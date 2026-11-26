//! `tatum set play <dir> --record <name>`: the take.
//!
//! A take is the set as it was played, in two files: `<name>.wav`, what came
//! out, and `<name>.txt`, every knob, pad, key, strip, step move and scene
//! the performer touched, each on the bar and the sample it landed on, in
//! the script `set render --perform` reads. Rendered again from the script,
//! the take comes back sample for sample.
//!
//! That cannot be had from the plain live session, where the main thread
//! plans what the controller does whenever it wakes and the audio callback
//! applies it at the top of whichever callback comes next: when a knob
//! lands, how a tempo ramp steps and when a step held for its line goes out
//! all follow the wall clock and the device's buffer. So a take is played
//! the way `set render` plays: a [`Stage`] moved on block by block on the
//! main thread, with what arrived from the controller handed in between two
//! blocks, a little ahead of the device. The audio callback only copies
//! samples out of a lock-free ring ([`Ring`]); the blocks also go, with the
//! script's lines, down a channel to a thread that writes the two files.
//! Nothing in the callback allocates, locks or touches a file, and a
//! dropout heard live (the main thread late, a step slow to build) is not
//! in the take: the take is the blocks, not what the device managed to play.
//!
//! The price is latency: what the controller does is heard after the
//! samples already waiting in the ring, about [`LOOKAHEAD`] frames.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use tatum_core::dsl::ast::KeyAction;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::include::Source;
use crate::live::{Status, Ui};
use crate::midi::{Event as Midi, DRUM_CHANNEL};
use crate::resample::Resampler;
use crate::set::{Act, Cue, TakeHeader};
use crate::stage::Stage;

/// Frames the session keeps rendered ahead of the device, at the least. A
/// callback bigger than half of it raises it to two callbacks and a block.
pub const LOOKAHEAD: usize = 1024;

// ── The ring the callback reads ─────────────────────────────────────────────

/// Stereo samples from one thread to one other, without a lock: the main
/// thread pushes rendered blocks, the audio callback pulls what it needs.
pub struct Ring {
    l: Box<[AtomicU32]>,
    r: Box<[AtomicU32]>,
    /// Frames ever pushed, and ever pulled. Only the pusher stores the first
    /// and only the puller the second.
    written: AtomicUsize,
    read: AtomicUsize,
}

impl Ring {
    pub fn new(frames: usize) -> Self {
        let n = frames.next_power_of_two();
        let slots = || (0..n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice();
        Self { l: slots(), r: slots(), written: AtomicUsize::new(0), read: AtomicUsize::new(0) }
    }

    /// Frames pushed and not yet pulled.
    pub fn waiting(&self) -> usize {
        self.written.load(Ordering::Acquire).wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// Frames there is room to push.
    pub fn room(&self) -> usize {
        self.l.len() - self.waiting()
    }

    /// Push a block. The caller has checked [`room`](Self::room); what does
    /// not fit is dropped rather than written over what waits.
    pub fn push(&self, l: &[f32], r: &[f32]) {
        let w = self.written.load(Ordering::Relaxed);
        let n = l.len().min(r.len()).min(self.room());
        let mask = self.l.len() - 1;
        for i in 0..n {
            let at = w.wrapping_add(i) & mask;
            self.l[at].store(l[i].to_bits(), Ordering::Relaxed);
            self.r[at].store(r[i].to_bits(), Ordering::Relaxed);
        }
        self.written.store(w.wrapping_add(n), Ordering::Release);
    }

    /// Pull up to `l.len()` frames. Returns how many there were; the rest of
    /// the buffers is left alone.
    pub fn pull(&self, l: &mut [f32], r: &mut [f32]) -> usize {
        let at = self.read.load(Ordering::Relaxed);
        let n = l.len().min(r.len()).min(self.written.load(Ordering::Acquire).wrapping_sub(at));
        let mask = self.l.len() - 1;
        for i in 0..n {
            let k = at.wrapping_add(i) & mask;
            l[i] = f32::from_bits(self.l[k].load(Ordering::Relaxed));
            r[i] = f32::from_bits(self.r[k].load(Ordering::Relaxed));
        }
        self.read.store(at.wrapping_add(n), Ordering::Release);
        n
    }
}

/// The audio callback's side: the ring, and the converter when the device
/// is not at the engine's rate.
pub struct Output {
    ring: Arc<Ring>,
    resampler: Option<Resampler>,
    /// Frames the ring did not have when the device asked: silence played.
    pub short: u64,
}

impl Output {
    pub fn new(ring: Arc<Ring>, resampler: Option<Resampler>) -> Self {
        Self { ring, resampler, short: 0 }
    }

    /// Fill one callback's interleaved stereo buffer. Allocates nothing.
    pub fn fill(&mut self, data: &mut [f32]) {
        let mut l = [0.0f32; BLOCK_SIZE];
        let mut r = [0.0f32; BLOCK_SIZE];
        for chunk in data.chunks_mut(BLOCK_SIZE * 2) {
            let frames = chunk.len() / 2;
            match self.resampler.as_mut() {
                None => {
                    let got = self.ring.pull(&mut l[..frames], &mut r[..frames]);
                    if got < frames {
                        l[got..frames].fill(0.0);
                        r[got..frames].fill(0.0);
                        self.short += (frames - got) as u64;
                    }
                }
                Some(rs) => {
                    while rs.needed(frames) > 0 && rs.can_push() {
                        let mut bl = [0.0f32; BLOCK_SIZE];
                        let mut br = [0.0f32; BLOCK_SIZE];
                        let got = self.ring.pull(&mut bl, &mut br);
                        self.short += (BLOCK_SIZE - got) as u64;
                        rs.push(&bl, &br);
                    }
                    rs.pull(&mut l[..frames], &mut r[..frames]);
                }
            }
            for (i, frame) in chunk.chunks_mut(2).enumerate() {
                frame[0] = l[i];
                if frame.len() > 1 {
                    frame[1] = r[i];
                }
            }
        }
    }
}

// ── What the controller sent, waiting for the next block ────────────────────

/// What arrived since the last block. Notes, pads and the computer's keys
/// keep their order; a knob turned fast is only its last value, and the
/// strip the same, as in the plain session.
pub struct Inbox {
    ordered: Vec<Act>,
    bend: Option<u16>,
    ccs: [Option<u8>; 128],
}

impl Default for Inbox {
    fn default() -> Self {
        Self { ordered: Vec::new(), bend: None, ccs: [None; 128] }
    }
}

impl Inbox {
    /// A message from the controller: the same path for a real one and for
    /// a test.
    pub fn midi(&mut self, event: Midi) {
        match event {
            Midi::Cc(cc, v) => self.ccs[(cc & 0x7f) as usize] = Some(v),
            Midi::Bend(v) => self.bend = Some(v),
            Midi::Note { channel, note, velocity } => self.ordered.push(if channel == DRUM_CHANNEL {
                Act::Pad(note, velocity)
            } else {
                Act::Key(note, velocity)
            }),
            Midi::Pressure { channel, note, value } => {
                self.ordered.push(Act::Pressure(channel == DRUM_CHANNEL, note, value))
            }
            Midi::Other { .. } => {}
        }
    }

    /// Something from the computer's keys or the screen.
    pub fn act(&mut self, act: Act) {
        self.ordered.push(act);
    }

    /// Everything waiting, in the order it acts: notes and keys as they
    /// came, then the strip, then the knobs.
    fn take(&mut self) -> Vec<Act> {
        let mut acts = std::mem::take(&mut self.ordered);
        acts.extend(self.bend.take().map(Act::Bend));
        for (cc, v) in self.ccs.iter_mut().enumerate() {
            if let Some(v) = v.take() {
                acts.push(Act::Cc(cc as u8, v));
            }
        }
        acts
    }
}

// ── The two files ───────────────────────────────────────────────────────────

enum ToDisk {
    Audio(Vec<f32>),
    Line(String),
}

/// Writes the take on a thread of its own: the WAV as it plays (its header
/// rewritten every few seconds, so a take cut short still opens), and the
/// script a line at a time.
pub struct Recorder {
    tx: Option<Sender<ToDisk>>,
    writer: Option<JoinHandle<Result<usize, String>>>,
    /// Interleaved frames not yet sent.
    pending: Vec<f32>,
    pub wav: PathBuf,
    pub txt: PathBuf,
}

/// `<name>.wav` and `<name>.txt` (a name given with either extension is the
/// same take), or why they cannot be written.
pub fn take_files(name: &Path) -> Result<(PathBuf, PathBuf), String> {
    let base = match name.extension() {
        Some(e) if e == "wav" || e == "txt" => name.with_extension(""),
        _ => name.to_path_buf(),
    };
    let (wav, txt) = (base.with_extension("wav"), base.with_extension("txt"));
    for p in [&wav, &txt] {
        if p.exists() {
            return Err(format!("{} exists; a take does not write over another, name this one otherwise", p.display()));
        }
    }
    Ok((wav, txt))
}

/// Frames sent to the writer at a time.
const CHUNK: usize = 8192;

impl Recorder {
    /// `<name>.wav` and `<name>.txt`, neither of which may exist: a take is
    /// not written over another.
    pub fn create(name: &Path, header: &[String]) -> Result<Self, String> {
        let (wav, txt) = take_files(name)?;
        let open = |p: &PathBuf| File::create(p).map_err(|e| format!("{}: {}", p.display(), e));
        let (mut audio, mut script) = (BufWriter::new(open(&wav)?), BufWriter::new(open(&txt)?));
        let sr = SAMPLE_RATE as u32;
        let wav_name = wav.display().to_string();
        let txt_name = txt.display().to_string();
        let lines = header.to_vec();
        let (tx, rx) = channel::<ToDisk>();
        let writer = std::thread::spawn(move || -> Result<usize, String> {
            let wav_err = |e: std::io::Error| format!("{}: {}", wav_name, e);
            let txt_err = |e: std::io::Error| format!("{}: {}", txt_name, e);
            audio.write_all(&tatum_core::wav::header_stereo_16(0, sr)).map_err(wav_err)?;
            for l in lines {
                writeln!(script, "{}", l).map_err(txt_err)?;
            }
            script.flush().map_err(txt_err)?;
            let mut frames = 0usize;
            let mut since_header = 0usize;
            let mut bytes = Vec::with_capacity(CHUNK * 4);
            for msg in rx {
                match msg {
                    ToDisk::Audio(samples) => {
                        bytes.clear();
                        for s in &samples {
                            bytes.extend_from_slice(&tatum_core::wav::sample_16(*s).to_le_bytes());
                        }
                        audio.write_all(&bytes).map_err(wav_err)?;
                        frames += samples.len() / 2;
                        since_header += samples.len() / 2;
                        if since_header >= 5 * SAMPLE_RATE as usize {
                            since_header = 0;
                            rewrite_header(&mut audio, frames, sr).map_err(wav_err)?;
                        }
                    }
                    ToDisk::Line(line) => {
                        writeln!(script, "{}", line).map_err(txt_err)?;
                        script.flush().map_err(txt_err)?;
                    }
                }
            }
            rewrite_header(&mut audio, frames, sr).map_err(wav_err)?;
            script.flush().map_err(txt_err)?;
            Ok(frames)
        });
        Ok(Self { tx: Some(tx), writer: Some(writer), pending: Vec::with_capacity(CHUNK * 2), wav, txt })
    }

    pub fn audio(&mut self, l: &[f32], r: &[f32]) {
        for (a, b) in l.iter().zip(r) {
            self.pending.push(*a);
            self.pending.push(*b);
        }
        if self.pending.len() >= CHUNK * 2 {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let full = std::mem::replace(&mut self.pending, Vec::with_capacity(CHUNK * 2));
        if let Some(tx) = &self.tx {
            let _ = tx.send(ToDisk::Audio(full));
        }
    }

    pub fn line(&mut self, line: String) {
        // In order with the audio around it, should anyone read both.
        self.flush();
        if let Some(tx) = &self.tx {
            let _ = tx.send(ToDisk::Line(line));
        }
    }

    /// Send what is left, close both files and wait for them. Returns the
    /// frames written.
    pub fn finish(mut self) -> Result<usize, String> {
        self.flush();
        drop(self.tx.take());
        match self.writer.take().map(|w| w.join()) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err(String::from("the thread writing the take panicked")),
            None => Ok(0),
        }
    }
}

fn rewrite_header(audio: &mut BufWriter<File>, frames: usize, sr: u32) -> std::io::Result<()> {
    audio.flush()?;
    let file = audio.get_mut();
    let end = file.stream_position()?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&tatum_core::wav::header_stereo_16(frames, sr))?;
    file.seek(SeekFrom::Start(end))?;
    Ok(())
}

/// The line a take writes for an act: `12.503906@529152  cc 74 64`, and
/// what it did, after a `#`, for the reader.
fn script_line(bar: f64, sample: usize, act: &Act, said: &str) -> String {
    let line = Cue { bar, sample: Some(sample), act: act.clone() }.line();
    if said.is_empty() {
        line
    } else {
        format!("{:<36}# {}", line, said.replace(['\n', '\r'], " "))
    }
}

// ── The session, without the device ─────────────────────────────────────────

/// What a block said, for the screen or the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// What a knob or the strip reads now.
    Reading(String),
    Info(String),
    Good(String),
}

/// A take being played: the stage, what waits for it, the ring and the
/// files. The device's callback and the controller are outside; a test
/// drives this the same way the session does.
pub struct Session {
    pub stage: Stage,
    pub inbox: Inbox,
    ring: Arc<Ring>,
    recorder: Option<Recorder>,
    /// Frames to keep in the ring.
    pub lookahead: usize,
}

impl Session {
    pub fn new(stage: Stage, ring: Arc<Ring>, recorder: Option<Recorder>) -> Self {
        Self { stage, inbox: Inbox::default(), ring, recorder, lookahead: LOOKAHEAD }
    }

    /// Render blocks until `lookahead` frames wait for the device. `each` is
    /// handed every block and the stage after it, for the screen's meters.
    pub fn render_ahead(&mut self, mut each: impl FnMut(&[f32], &[f32], &mut Stage)) -> Result<Vec<Said>, String> {
        let mut said = Vec::new();
        while self.ring.waiting() < self.lookahead && self.ring.room() >= BLOCK_SIZE {
            let (l, r) = self.block(&mut said)?;
            each(&l, &r, &mut self.stage);
        }
        Ok(said)
    }

    /// One block: what waits acts on its first sample, then it renders.
    fn block(&mut self, said: &mut Vec<Said>) -> Result<([f32; BLOCK_SIZE], [f32; BLOCK_SIZE]), String> {
        let acts = self.inbox.take();
        let (bar, sample) = (self.stage.position(), self.stage.frames);
        for act in &acts {
            let s = self.stage.act(act);
            if let Some(rec) = self.recorder.as_mut() {
                rec.line(script_line(bar, sample, act, &s));
            }
            if !s.is_empty() {
                said.push(match act {
                    Act::Cc(..) | Act::Bend(_) => Said::Reading(s),
                    _ => Said::Info(s),
                });
            }
        }
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let turn = self.stage.block(&mut l, &mut r)?;
        said.extend(turn.tick.into_iter().filter(|s| !s.starts_with("now ")).map(Said::Info));
        said.extend(turn.log.into_iter().map(Said::Good));
        self.ring.push(&l, &r);
        if let Some(rec) = self.recorder.as_mut() {
            rec.audio(&l, &r);
        }
        // Engines the player is done with are freed here, off the device's
        // thread, as the plain session frees them on its main thread.
        while self.stage.player.take_retired().is_some() {}
        Ok((l, r))
    }

    /// The take is over: its `end` on the sample it stopped on, and both
    /// files closed. Returns the files and the frames written.
    pub fn finish(mut self) -> Result<Option<(PathBuf, PathBuf, usize)>, String> {
        let Some(mut rec) = self.recorder.take() else { return Ok(None) };
        rec.line(script_line(self.stage.position(), self.stage.frames, &Act::End, ""));
        let (wav, txt) = (rec.wav.clone(), rec.txt.clone());
        let frames = rec.finish()?;
        Ok(Some((wav, txt, frames)))
    }
}

// ── The live session ────────────────────────────────────────────────────────

/// What the callback counts for the summary.
#[derive(Default)]
struct Counters {
    callbacks: AtomicU64,
    frames: AtomicU64,
    /// The most frames one callback asked for.
    widest: AtomicU64,
    worst_ns: AtomicU64,
    late: AtomicU64,
    short: AtomicU64,
}

/// Play the set and record it. `walk` and `header` are how it is played.
#[allow(clippy::too_many_arguments)]
pub fn run(
    steps: Vec<crate::set::Step>,
    walk: &crate::set::Walk,
    header: &TakeHeader,
    name: &Path,
    device_name: Option<&str>,
    forced_rate: Option<u32>,
    midi_name: Option<&str>,
    tui: bool,
    glass: bool,
) -> Result<(), String> {
    let set_name = steps[0].path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    // Said before the set is measured, which takes a while.
    take_files(name)?;
    let mut stage = Stage::new(steps, walk)?;
    // A performer counts bars from 1.
    stage.script_count = false;
    let device = crate::live::open_device(device_name)?;
    let (mut config, rate) = crate::live::pick_config(&device, forced_rate)?;
    if let Some(&frames) = crate::live::BUFFER.get() {
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    let resampler = if rate == SAMPLE_RATE as u32 { None } else { Some(Resampler::new(SAMPLE_RATE as u32, rate)) };
    let device_desc = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "?".into());
    // Room for the biggest buffer `--buffer` allows, twice.
    let ring = Arc::new(Ring::new(8192 * 2 + LOOKAHEAD));
    let counters = Arc::new(Counters::default());
    let counters_cb = Arc::clone(&counters);
    let mut output = Output::new(Arc::clone(&ring), resampler);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let t0 = Instant::now();
                let before = output.short;
                output.fill(data);
                let frames = (data.len() / 2) as u64;
                counters_cb.callbacks.fetch_add(1, Ordering::Relaxed);
                counters_cb.frames.fetch_add(frames, Ordering::Relaxed);
                counters_cb.widest.fetch_max(frames, Ordering::Relaxed);
                counters_cb.short.fetch_add(output.short - before, Ordering::Relaxed);
                let ns = t0.elapsed().as_nanos() as u64;
                counters_cb.worst_ns.fetch_max(ns, Ordering::Relaxed);
                if ns > frames * 1_000_000_000 / rate as u64 {
                    counters_cb.late.fetch_add(1, Ordering::Relaxed);
                }
            },
            |_err| {
                crate::live::STREAM_ERRORS.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("cannot open output stream: {}", e))?;
    let (midi_tx, midi_rx) = channel::<Midi>();
    let inputs = crate::midi::open(midi_name, &midi_tx)?;
    // The files last: nothing that can still fail leaves an empty take behind.
    let lines = vec![
        format!("# a take of {}, from `tatum set play --record`", set_name),
        String::from("# set render <dir> --perform <this file> plays it back, sample for sample"),
        header.line(),
    ];
    let recorder = Recorder::create(name, &lines)?;
    let (wav, txt) = (recorder.wav.clone(), recorder.txt.clone());
    let mut session = Session::new(stage, Arc::clone(&ring), Some(recorder));
    let telemetry = tui.then(|| Arc::new(crate::tui::Telemetry::new()));
    let meter = |l: &[f32], r: &[f32], stage: &mut Stage| {
        if let Some(t) = &telemetry {
            t.push(l, r);
            if let Some(e) = stage.player.engine_mut() {
                e.set_band_metering(true);
                t.meter(e);
            }
        }
    };
    // Sound waiting before the device asks for any.
    session.render_ahead(meter)?;

    stream.play().map_err(|e| format!("cannot start output stream: {}", e))?;

    let (key_tx, key_rx) = channel::<crate::keys::Key>();
    let mut ui = match &telemetry {
        Some(t) => {
            let mut screen = crate::tui::Screen::new(Arc::clone(t), device_desc.clone(), inputs.names.clone())
                .map_err(|e| format!("cannot open the screen: {}", e))?;
            screen.glass = glass;
            screen.say(format!("recording a take to {}", wav.display()), crate::tui::Tone::Info);
            Ui::Screen(Box::new(screen))
        }
        None => Ui::Plain(Status::new()),
    };
    let _keys = if let Ui::Plain(_) = ui {
        let nav = &session.stage.nav;
        eprintln!("playing {} on {}, recording to {} and {}", set_name, device_desc, wav.display(), txt.display());
        if !inputs.names.is_empty() {
            eprintln!("  midi: {}", inputs.names.join(", "));
        }
        eprintln!("  steps, {} bars to a phrase; a move lands on the next phrase line:", nav.phrase);
        for i in 0..nav.steps.len() {
            eprintln!("    {}", nav.describe(i));
        }
        eprintln!("  space or → next, ← back, 1-9 a step, q quits and closes the take; the files are not watched");
        crate::live::print_keyboard(&session.stage.planner);
        Some(crate::keys::spawn(key_tx))
    } else {
        None
    };
    let song = |nav: &crate::setnav::SetNav, ui: &mut Ui| {
        if let Ok(s) = Source::load(&nav.path()) {
            ui.song(&nav.describe(nav.current), &s);
        }
    };
    song(&session.stage.nav, &mut ui);

    let started = Instant::now();
    let poll_every = Duration::from_millis(if ui.is_screen() { 33 } else { 50 });
    let mut next_poll = Instant::now() + poll_every;
    let mut shown_step = session.stage.nav.current;
    let mut knob_moves = 0u32;
    let result = loop {
        // A short wait: the ring is refilled every time round.
        let first = match midi_rx.recv_timeout(Duration::from_millis(2)) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        };
        let now_s = started.elapsed().as_secs_f32();
        for event in first.into_iter().chain(std::iter::from_fn(|| midi_rx.try_recv().ok())) {
            if let Ui::Screen(screen) = &mut ui {
                let planner = &session.stage.planner;
                let what = planner.song().and_then(|s| crate::midi::meaning(s, Some(planner), &event));
                screen.midi_in(now_s, crate::midi::describe(&event), what);
            }
            session.inbox.midi(event);
        }
        // Two callbacks and a block ahead of the widest the device asked for.
        let widest = counters.widest.load(Ordering::Relaxed) as usize;
        session.lookahead = LOOKAHEAD.max(widest * 2 + BLOCK_SIZE);
        let said = match session.render_ahead(meter) {
            Ok(s) => s,
            Err(e) => break Err(e),
        };
        let mut readings = Vec::new();
        for s in said {
            match s {
                Said::Reading(r) => {
                    knob_moves += 1;
                    readings.push(r);
                }
                Said::Info(line) => ui.say(now_s, &line, crate::tui::Tone::Info),
                Said::Good(line) => ui.say(now_s, &line, crate::tui::Tone::Good),
            }
        }
        if !readings.is_empty() {
            ui.readings(readings);
        }
        if Instant::now() < next_poll {
            continue;
        }
        next_poll = Instant::now() + poll_every;

        let mut quit = false;
        let mut keys: Vec<crate::keys::Key> = key_rx.try_iter().collect();
        if let Ui::Screen(screen) = &mut ui {
            for k in screen.keys() {
                match k {
                    crate::tui::Key::Quit => keys.push(crate::keys::Key::Quit),
                    crate::tui::Key::Next => keys.push(crate::keys::Key::Next),
                    crate::tui::Key::Prev => keys.push(crate::keys::Key::Prev),
                    crate::tui::Key::Step(n) => keys.push(crate::keys::Key::Step(n)),
                    crate::tui::Key::Loop => keys.push(crate::keys::Key::Loop),
                    crate::tui::Key::LoopUp => keys.push(crate::keys::Key::LoopUp),
                    crate::tui::Key::Keep => keys.push(crate::keys::Key::Keep),
                    crate::tui::Key::Named(n) => keys.push(crate::keys::Key::Named(n)),
                    crate::tui::Key::Mute => {
                        for (name, _) in screen.selected_tracks() {
                            session.inbox.act(Act::Mute(name));
                        }
                    }
                    crate::tui::Key::Solo => {
                        let names: Vec<String> = screen.selected_tracks().into_iter().map(|(n, _)| n).collect();
                        if !names.is_empty() {
                            session.inbox.act(Act::Solo(names));
                        }
                    }
                    // The take plays the files as they are: an edit would
                    // be heard and not be in it.
                    crate::tui::Key::Edit(_) | crate::tui::Key::Knob(_) | crate::tui::Key::Undo => {
                        screen.say("not while recording: a take plays the files as they are", crate::tui::Tone::Bad)
                    }
                }
            }
            screen.soloed = session.stage.planner.soloed().to_vec();
        }
        let (bindings, _) = session.stage.planner.keyboard();
        for key in keys {
            let key = match key {
                crate::keys::Key::Named(name) => {
                    match bindings.iter().find(|b| b.key == name.word()).map(|b| &b.action) {
                        Some(KeyAction::Reset) => {
                            session.inbox.act(Act::Reset);
                            continue;
                        }
                        Some(KeyAction::Voice(mv)) => {
                            session.inbox.act(Act::Voice(mv.clone()));
                            continue;
                        }
                        Some(KeyAction::Perform(scene)) => {
                            session.inbox.act(Act::Perform(scene.clone()));
                            continue;
                        }
                        Some(KeyAction::Next) => crate::keys::Key::Next,
                        Some(KeyAction::Prev) => crate::keys::Key::Prev,
                        Some(KeyAction::Step(n)) => crate::keys::Key::Step(*n),
                        Some(KeyAction::Loop) => crate::keys::Key::Loop,
                        None => match name.default_move() {
                            Some(k) => k,
                            None => continue,
                        },
                    }
                }
                k => k,
            };
            match key {
                crate::keys::Key::Quit => quit = true,
                crate::keys::Key::Next => session.inbox.act(Act::Next),
                crate::keys::Key::Prev => session.inbox.act(Act::Prev),
                crate::keys::Key::Step(n) => session.inbox.act(Act::Step(n)),
                crate::keys::Key::Loop => session.inbox.act(Act::Loop(true)),
                crate::keys::Key::LoopUp => session.inbox.act(Act::Loop(false)),
                crate::keys::Key::Keep => session.inbox.act(Act::Keep),
                crate::keys::Key::Named(_) => {}
            }
        }
        if quit {
            break Ok(());
        }
        let stage = &session.stage;
        let nav = &stage.nav;
        if nav.current != shown_step {
            shown_step = nav.current;
            song(nav, &mut ui);
        }
        let bar = stage.bar();
        if let Some((step, bars)) = nav.waiting(bar) {
            let line = format!(
                "bar {} · next {} in {} bar{}",
                bar + 1,
                nav.describe(step),
                bars,
                if bars == 1 { "" } else { "s" }
            );
            ui.waiting(&line);
        }
        if let Ui::Screen(screen) = &mut ui {
            let planner = &stage.planner;
            screen.bound = bindings.iter().map(|b| b.key.clone()).collect();
            screen.voice = planner.voice_name();
            screen.pages = planner.pages();
            screen.pad_voices = planner.waiting_for_pads();
            screen.perform = planner.scene().map(String::from);
            let step = &nav.steps[nav.current];
            screen.set = Some(crate::tui::SetView {
                steps: nav.steps.iter().map(|s| s.name()).collect(),
                current: nav.current,
                next: nav.waiting(bar),
                phrase: nav.phrase,
                bar: nav.bar_in_step(bar),
                bars: step.length(),
                cues: step.cues.clone(),
                moves_in: nav.moves_in(bar),
                capped: step.max.filter(|_| step.auto.is_none()),
                looping: nav.looping,
                pass: nav.pass(bar),
                bar_loop: nav.loop_view(),
            });
            if let Some(e) = stage.player.engine() {
                screen.position(e.current_bar(), e.global_step(), e.tempo());
            }
            if let Err(e) = screen.draw() {
                break Err(format!("cannot draw the screen: {}", e));
            }
        }
    };
    drop(inputs);
    drop(stream);
    drop(ui);
    let closed = session.finish();
    result?;
    let (wav, txt, frames) = closed?.expect("a live take records");

    let callbacks = counters.callbacks.load(Ordering::Relaxed);
    let played = counters.frames.load(Ordering::Relaxed);
    let worst = counters.worst_ns.load(Ordering::Relaxed) as f64 / 1e6;
    let per_cb = if callbacks > 0 { played as f64 / callbacks as f64 } else { 0.0 };
    eprintln!(
        "played {:.1} s in {} callbacks of ~{:.0} frames, worst {:.3} ms, {} late",
        played as f64 / rate as f64,
        callbacks,
        per_cb,
        worst,
        counters.late.load(Ordering::Relaxed)
    );
    let short = counters.short.load(Ordering::Relaxed);
    if short > 0 {
        eprintln!(
            "{:.2} s of silence played where the set was not rendered in time (the take has no gap: it is the set, not the device)",
            short as f64 / rate as f64
        );
    }
    if knob_moves > 0 {
        eprintln!("{} knob moves", knob_moves);
    }
    let secs = frames as f32 / SAMPLE_RATE;
    eprintln!("wrote {} ({}:{:02}) and {}", wav.display(), (secs / 60.0) as u32, (secs % 60.0) as u32, txt.display());
    eprintln!("  tatum set render {} --perform {} -o again.wav  renders it again", set_name, txt.display());
    Ok(())
}

#[cfg(test)]
mod tests;
