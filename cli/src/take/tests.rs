//! A take played offline: no device and no controller, but the same path.
//! Messages go into the session's [`Inbox`] as the MIDI thread hands them
//! over, the session renders ahead into the [`Ring`] as the main thread
//! does, and an [`Output`] pulls it out a callback at a time, at the sizes
//! devices ask for. The take's script is then rendered by `set render
//! --perform` and has to be the take, sample for sample.

use super::*;
use crate::set::{load, parse_script, render_set, Act, Cue, TakeHeader};
use tatum_core::dsl::ast::VoiceMove;

const RIG: &str = r#"
tempo 120
scale C minor
module bass low { cutoff 0.5 sustain 0.8 }
module fm pluck_v { }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
pattern krest { -*16 }
track bass { play line using low level 0.5 out > lowpass(600, 0.1, wet=0.0) as lp > master }
track lead { play krest using pluck_v level 0.4 out > master }
master { in > out }
midi {
    zone lead 60..84 > lead
    pad 41 > hold bass
    pad 42 > loop
    cc 30 > bass level
    cc 31 > bass cutoff 200hz..3khz
}
perform drop {
    set bass level 0.2
}
keyboard {
    f1 > perform drop
    quantize bar
}
"#;

/// Frames in a bar at 120 BPM.
const BAR: usize = (SAMPLE_RATE * 2.0) as usize;

/// A fresh directory: a set of three steps (the rig; the rig with only a
/// value changed, which moves on by itself after two bars; the rig faster
/// with a track more, which needs a new engine and a tempo ramp), and room
/// for the take's files.
fn set_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tatum-take-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let set = dir.join("set");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join("01.synth"), format!("# set: bars=4\n{RIG}")).unwrap();
    std::fs::write(set.join("02.synth"), format!("# set: bars=2 auto\n{}", RIG.replace("cutoff 0.5", "cutoff 0.7")))
        .unwrap();
    let faster = RIG.replace("tempo 120", "tempo 130") + "track echo { play line using low level 0.2 out > master }\n";
    std::fs::write(set.join("03.synth"), format!("# set: bars=4\n{faster}")).unwrap();
    dir
}

fn header(blend: f32) -> TakeHeader {
    TakeHeader { phrase: 2, ramp_bars: 2.0, blend_bars: blend, auto: false, bars: 32, steps: None }
}

/// Something that arrives from outside: a controller's message, or the
/// computer's keys.
enum In {
    Midi(Midi),
    Act(Act),
}

fn cc(n: u8, v: u8) -> In {
    In::Midi(Midi::Cc(n, v))
}

fn pad(n: u8, v: u8) -> In {
    In::Midi(Midi::Note { channel: DRUM_CHANNEL, note: n, velocity: v })
}

fn key(n: u8, v: u8) -> In {
    In::Midi(Midi::Note { channel: 1, note: n, velocity: v })
}

struct Took {
    /// Every block the session rendered, as rendered.
    l: Vec<f32>,
    r: Vec<f32>,
    /// What the callbacks handed the device.
    heard_l: Vec<f32>,
    short: u64,
    wav: PathBuf,
    txt: PathBuf,
}

/// Play a take of the set in `dir` with callbacks of `cb` frames until
/// `frames` have been heard. Before every block, `at` is asked what arrives
/// now, given the frames heard and the stage about to render.
fn play(
    dir: &Path,
    head: &TakeHeader,
    cb: usize,
    frames: usize,
    meter: bool,
    mut at: impl FnMut(usize, &Stage) -> Vec<In>,
) -> Took {
    let steps = load(&dir.join("set"), head.bars).unwrap();
    let mut stage = Stage::new(steps, &head.walk()).unwrap();
    stage.script_count = false;
    let rec = Recorder::create(&dir.join(format!("take{cb}")), &[head.line()]).unwrap();
    let ring = Arc::new(Ring::new(8192 * 2 + LOOKAHEAD));
    let mut session = Session::new(stage, Arc::clone(&ring), Some(rec));
    session.lookahead = LOOKAHEAD.max(cb * 2 + BLOCK_SIZE);
    let mut output = Output::new(ring, None);
    let telemetry = crate::tui::Telemetry::new();
    let (mut l, mut r, mut heard_l) = (Vec::new(), Vec::new(), Vec::new());
    let mut data = vec![0.0f32; cb * 2];
    while heard_l.len() < frames {
        // The main thread's round: what arrived, then render ahead, a block
        // at a time so `at` sees every block coming.
        loop {
            for input in at(heard_l.len(), &session.stage) {
                match input {
                    In::Midi(m) => session.inbox.midi(m),
                    In::Act(a) => session.inbox.act(a),
                }
            }
            let lookahead = session.lookahead;
            session.lookahead = (ring_waiting(&session) + BLOCK_SIZE).min(lookahead);
            let mut rendered = 0;
            session
                .render_ahead(|bl, br, stage| {
                    rendered += 1;
                    l.extend_from_slice(bl);
                    r.extend_from_slice(br);
                    if meter {
                        telemetry.push(bl, br);
                        if let Some(e) = stage.player.engine_mut() {
                            e.set_band_metering(true);
                            telemetry.meter(e);
                        }
                    }
                })
                .unwrap();
            session.lookahead = lookahead;
            if rendered == 0 {
                break;
            }
        }
        // The device's callback.
        output.fill(&mut data);
        heard_l.extend(data.chunks(2).map(|f| f[0]));
    }
    let (wav, txt, written) = session.finish().unwrap().unwrap();
    assert_eq!(written, l.len(), "the WAV has every block rendered");
    Took { l, r, heard_l, short: output.short, wav, txt }
}

fn ring_waiting(s: &Session) -> usize {
    s.ring.waiting()
}

/// The WAV's samples, left and right.
fn read_wav(path: &Path) -> (Vec<i16>, Vec<i16>) {
    let bytes = std::fs::read(path).unwrap();
    let frames = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize / 4;
    assert_eq!(bytes.len(), 44 + frames * 4, "the header says how long the data is");
    let s: Vec<i16> = bytes[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    (s.iter().step_by(2).copied().collect(), s.iter().skip(1).step_by(2).copied().collect())
}

/// The take's script, rendered by `set render --perform`.
fn render_take(dir: &Path, txt: &Path) -> (crate::set::Script, crate::set::Rendered) {
    let script = parse_script(&std::fs::read_to_string(txt).unwrap()).unwrap();
    let take = script.take.clone().expect("a take says how it was played");
    let steps = load(&dir.join("set"), take.bars).unwrap();
    let rendered = render_set(steps, &take.walk(), &script.cues).unwrap();
    (script, rendered)
}

/// The proof: what the session rendered is what the device was handed, is
/// what the WAV holds, and is what the script renders to, sample for sample.
fn assert_same(dir: &Path, took: &Took) -> (crate::set::Script, crate::set::Rendered) {
    assert_eq!(took.short, 0, "the device was never short");
    let n = took.heard_l.len();
    assert!(n <= took.l.len());
    assert!(took.heard_l.iter().zip(&took.l).all(|(a, b)| a.to_bits() == b.to_bits()), "the device heard the blocks");
    let (wl, wr) = read_wav(&took.wav);
    let pcm = |x: &[f32]| x.iter().map(|s| tatum_core::wav::sample_16(*s)).collect::<Vec<_>>();
    assert!(wl == pcm(&took.l) && wr == pcm(&took.r), "the WAV is the blocks");
    let (script, rendered) = render_take(dir, &took.txt);
    assert_eq!(rendered.l.len(), took.l.len(), "the render is as long as the take");
    let first = |a: &[f32], b: &[f32]| a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits());
    assert_eq!(first(&rendered.l, &took.l), None, "left differs from the take at this sample");
    assert_eq!(first(&rendered.r, &took.r), None, "right differs from the take at this sample");
    let bytes = tatum_core::wav::encode_stereo_16(&rendered.l, &rendered.r, SAMPLE_RATE as u32);
    assert!(bytes == std::fs::read(&took.wav).unwrap(), "`set render` writes the take's WAV byte for byte");
    (script, rendered)
}

/// A performance: knobs, a pad held, keys in the lead zone, the strip,
/// aftertouch, a scene, a step asked for from the computer's keys, a loop
/// of bars, a page and a mute, spread over a step that changes values, one
/// that moves on by itself, and one that needs a new engine and ramps.
/// Each gesture once, in the first round after its frame has been heard.
fn performance() -> impl FnMut(usize, &Stage) -> Vec<In> {
    let mut done = 0;
    move |heard, _| {
        let marks = gestures();
        let mut out = Vec::new();
        while done < marks.len() && heard >= marks[done].0 {
            out.extend(marks[done].1());
            done += 1;
        }
        out
    }
}

type Gesture = (usize, fn() -> Vec<In>);

fn gestures() -> [Gesture; 12] {
    [
        (BAR / 4, || vec![cc(30, 20), cc(31, 90)]),
        (BAR / 3, || vec![cc(30, 40)]),
        (BAR / 2, || vec![pad(41, 110)]),
        (BAR * 3 / 4, || vec![key(62, 100), In::Midi(Midi::Bend(12000))]),
        (BAR, || {
            vec![
                key(62, 0),
                In::Midi(Midi::Pressure { channel: DRUM_CHANNEL, note: Some(41), value: 90 }),
                In::Midi(Midi::Pressure { channel: 1, note: None, value: 40 }),
            ]
        }),
        (BAR + BAR / 2, || vec![In::Act(Act::Next), pad(41, 0), In::Midi(Midi::Bend(8192))]),
        (BAR * 2 + 1000, || vec![In::Act(Act::Perform(String::from("drop"))), cc(31, 10)]),
        (BAR * 3, || vec![pad(42, 100), In::Act(Act::Voice(VoiceMove::Next))]),
        (BAR * 4 + 500, || vec![pad(42, 0), In::Act(Act::Mute(String::from("lead")))]),
        (BAR * 5 + 7, || vec![In::Act(Act::Loop(true)), cc(30, 127)]),
        (BAR * 5 + BAR / 2, || vec![In::Act(Act::Loop(true)), In::Act(Act::Solo(vec![String::from("bass")]))]),
        (BAR * 6, || vec![cc(31, 64), In::Midi(Midi::Other { bytes: [0xc0, 3, 0], len: 2 })]),
    ]
}

/// The take is the render of its script, sample for sample, whatever size
/// of callback the device asked for, the screen's meters on or off.
#[test]
fn a_take_renders_back_sample_for_sample() {
    for (cb, meter) in [(64, false), (256, true), (512, false), (441, false)] {
        let dir = set_dir();
        let took = play(&dir, &header(0.0), cb, BAR * 7, meter, performance());
        let (script, rendered) = assert_same(&dir, &took);
        // Everything played is in it, and it walked the whole set.
        let acts: Vec<&Act> = script.cues.iter().map(|c| &c.act).collect();
        for want in [
            Act::Cc(30, 20),
            Act::Pad(41, 110),
            Act::Key(62, 100),
            Act::Bend(12000),
            Act::Pressure(true, Some(41), 90),
            Act::Pressure(false, None, 40),
            Act::Next,
            Act::Perform(String::from("drop")),
            Act::Voice(VoiceMove::Next),
            Act::Mute(String::from("lead")),
            Act::Loop(true),
            Act::Solo(vec![String::from("bass")]),
            Act::End,
        ] {
            assert!(acts.contains(&&want), "callback {cb}: {want:?} is not in the take: {acts:?}");
        }
        let names: Vec<&str> = rendered.applied.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["01.synth", "02.synth", "03.synth"], "callback {cb}");
        assert!(took.l.iter().any(|v| v.abs() > 0.01), "callback {cb}: the take is silent");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Nobody touched anything: the script is its header and its `end`, and
/// still renders to the take.
#[test]
fn an_empty_take_is_the_set_left_alone() {
    let dir = set_dir();
    let took = play(&dir, &header(0.0), 256, BAR * 2, false, |_, _| Vec::new());
    let (script, _) = assert_same(&dir, &took);
    assert_eq!(script.cues.len(), 1, "{:?}", script.cues);
    assert_eq!(script.cues[0].act, Act::End);
    assert_eq!(script.cues[0].sample, Some(took.l.len()));
    assert_eq!(script.take, Some(header(0.0)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What arrives before the first block acts on the very first sample.
#[test]
fn an_act_before_the_first_block_lands_on_sample_zero() {
    let dir = set_dir();
    let took = play(&dir, &header(0.0), 512, BAR, false, |heard, stage| {
        if heard == 0 && stage.frames == 0 {
            vec![cc(30, 5), pad(41, 100)]
        } else {
            Vec::new()
        }
    });
    let (script, _) = assert_same(&dir, &took);
    // A note before a knob, as the inbox orders a block's arrivals.
    assert_eq!(script.cues[0], Cue { bar: 0.0, sample: Some(0), act: Act::Pad(41, 100) });
    assert_eq!(script.cues[1], Cue { bar: 0.0, sample: Some(0), act: Act::Cc(30, 5) });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Knobs and pads every block across both step changes -- one that only
/// changes values, held for its line, and one that swaps in a new engine
/// and ramps the tempo -- with and without a blend: each lands on the block
/// it landed on live, the blocks the steps land on included.
#[test]
fn knobs_through_a_step_change_and_a_hot_swap() {
    for blend in [0.0, 1.0] {
        let dir = set_dir();
        let mut n = 0u8;
        let mut asked = false;
        let took = play(&dir, &header(blend), 441, BAR * 6, false, |_, stage| {
            let mut out = Vec::new();
            let bar = stage.bar();
            if !asked && bar == 1 {
                asked = true;
                out.push(In::Act(Act::Next));
            }
            // Bars 1 to 5: step two lands on 2, step three on 4.
            if (1..5).contains(&bar) {
                n = n.wrapping_add(1);
                out.push(cc(30, n % 128));
                out.push(cc(31, 127 - n % 128));
                if n.is_multiple_of(16) {
                    out.push(pad(41, if n.is_multiple_of(32) { 0 } else { 100 }));
                }
            }
            out
        });
        let (script, rendered) = assert_same(&dir, &took);
        assert_eq!(rendered.applied.len(), 3, "blend {blend}: {:?}", rendered.applied);
        for (name, at) in &rendered.applied[1..] {
            assert!(
                script.cues.iter().any(|c| c.sample == Some(*at) && matches!(c.act, Act::Cc(30, _))),
                "blend {blend}: no knob on the block {name} landed on ({at})"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Every act a take writes reads back as itself, on its bar and sample; the
/// header too; and what a reader is told after the `#` is not read.
#[test]
fn a_take_script_reads_back_as_written() {
    let acts = vec![
        Act::Cc(74, 64),
        Act::Pad(44, 110),
        Act::Pad(44, 0),
        Act::Key(48, 100),
        Act::Bend(16383),
        Act::Next,
        Act::Prev,
        Act::Step(12),
        Act::Perform(String::from("drop")),
        Act::Pressure(true, Some(40), 90),
        Act::Pressure(false, None, 3),
        Act::Voice(VoiceMove::Next),
        Act::Voice(VoiceMove::Prev),
        Act::Voice(VoiceMove::To(String::from("bass"))),
        Act::Reset,
        Act::Loop(true),
        Act::Loop(false),
        Act::Keep,
        Act::Mute(String::from("drums")),
        Act::Solo(vec![String::from("bass"), String::from("lead")]),
        Act::End,
    ];
    let head = TakeHeader { phrase: 8, ramp_bars: 2.5, blend_bars: 4.0, auto: true, bars: 16, steps: Some(0xfeed) };
    let mut text = head.line() + "\n";
    let cues: Vec<Cue> = acts
        .iter()
        .enumerate()
        .map(|(i, a)| Cue { bar: i as f64 * 1.37 + 0.123_456_7, sample: Some(i * 1000 + 3), act: a.clone() })
        .collect();
    for c in &cues {
        text += &script_line(c.bar, c.sample.unwrap(), &c.act, "said # with a hash, and C#3");
        text += "\n";
    }
    let read = parse_script(&text).unwrap();
    assert_eq!(read.take, Some(head));
    assert_eq!(read.cues.len(), cues.len());
    for (a, b) in read.cues.iter().zip(&cues) {
        assert_eq!((&a.act, a.sample), (&b.act, b.sample));
        assert!((a.bar - b.bar).abs() < 1e-6, "{} against {}", a.bar, b.bar);
    }
    // A take's lines keep their order even where a loop of bars sends the
    // bar count back; a script without samples is ordered by bar, as ever.
    let looped = parse_script("8.5@100 cc 1 1\n6.0@200 cc 1 2\n").unwrap();
    assert_eq!(looped.cues.iter().map(|c| c.sample).collect::<Vec<_>>(), [Some(100), Some(200)]);
    let old = parse_script("8.5 cc 1 1\n6.0 cc 1 2\n").unwrap();
    assert_eq!(old.cues[0], Cue::at(6.0, Act::Cc(1, 2)));
    for bad in ["1@x cc 1 1", "1@-3 cc 1 1", "1 loop", "1 solo", "1 mute", "1 press pad", "# take: phrase=0"] {
        assert!(parse_script(bad).is_err(), "{bad}");
    }
}

/// The ring hands over what was pushed, in order, across its wrap, and the
/// callback plays silence, counted, when it runs dry.
#[test]
fn the_ring_wraps_and_runs_dry_quietly() {
    let ring = Arc::new(Ring::new(256));
    let mut output = Output::new(Arc::clone(&ring), None);
    let mut next = 0.0f32;
    let mut expect = 0.0f32;
    for _ in 0..40 {
        while ring.room() >= 100 {
            let l: Vec<f32> = (0..100).map(|i| next + i as f32).collect();
            ring.push(&l, &l);
            next += 100.0;
        }
        let mut data = [0.0f32; 2 * 77];
        output.fill(&mut data);
        for f in data.chunks(2) {
            assert_eq!(f[0], expect);
            assert_eq!(f[1], expect);
            expect += 1.0;
        }
    }
    assert_eq!(output.short, 0);
    let mut data = [1.0f32; 2 * 512];
    output.fill(&mut data);
    assert!(output.short > 0 && data[2 * 511] == 0.0);
}

/// A take is not written over another, whichever of its names is given.
#[test]
fn a_take_does_not_write_over_another() {
    let dir = set_dir();
    let rec = Recorder::create(&dir.join("one"), &[]).unwrap();
    rec.finish().unwrap();
    for name in ["one", "one.wav", "one.txt"] {
        let err = Recorder::create(&dir.join(name), &[]).err().expect("refused");
        assert!(err.contains("exists"), "{err}");
    }
    assert_eq!(take_files(&dir.join("two.wav")).unwrap(), (dir.join("two.wav"), dir.join("two.txt")));
    let _ = std::fs::remove_dir_all(&dir);
}
