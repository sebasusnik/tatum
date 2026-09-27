//! Moving through a set while it plays, for `tatum set play`.
//!
//! The performer asks for a step (a pad, the space bar, a number) and it
//! comes in on the next phrase line, not at once: the set keeps its phrasing
//! whoever is at the controls. A step that needs a new engine is built during
//! the last bar of the phrase and handed over on the bar line, the way a save
//! is; one that only changes values is held and sent on the line itself. A
//! step at another tempo starts at the tempo that was playing and ramps to
//! its own over a few bars, so the floor never hears the tempo jump.

use std::path::PathBuf;

use tatum_core::live::{FastOp, Generation, LivePlanner, Plan};

use crate::include::Source;
use crate::set::Step;

/// Where the set is asked to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Next,
    Prev,
    /// By position, counting from 1.
    To(usize),
}

struct Ramp {
    from: f32,
    to: f32,
    /// Seconds into the session, as the caller counts them.
    start: f32,
    secs: f32,
    generation: Generation,
}

pub struct SetNav {
    pub steps: Vec<Step>,
    /// The step playing.
    pub current: usize,
    /// Asked for and not yet sent.
    queued: Option<usize>,
    /// Bars in a phrase: a step lands on a bar that is a multiple of this.
    pub phrase: usize,
    /// Bars a tempo change takes. 0 jumps.
    pub ramp_bars: f32,
    /// Bars the outgoing step keeps playing under a new engine, DJ style; a
    /// step's own `# set: blend=` wins. 0 hands over on the line.
    pub blend_bars: f32,
    /// A step that only changes values, waiting for the phrase line.
    held: Option<(Plan, usize, f32, f32)>,
    /// A swap sent and not yet landed: the step, and the tempo to ramp from and to.
    in_flight: Option<(usize, f32, f32)>,
    ramp: Option<Ramp>,
    /// The bar a queued step was last built on, so it is built once.
    built_on: Option<usize>,
}

impl SetNav {
    pub fn new(steps: Vec<Step>, phrase: usize, ramp_bars: f32) -> Self {
        Self {
            steps,
            current: 0,
            queued: None,
            phrase: phrase.max(1),
            ramp_bars,
            blend_bars: 0.0,
            held: None,
            in_flight: None,
            ramp: None,
            built_on: None,
        }
    }

    pub fn path(&self) -> PathBuf {
        self.steps[self.current].path.clone()
    }

    /// `3/20 001.synth murk` for a step.
    pub fn describe(&self, i: usize) -> String {
        let s = &self.steps[i];
        let mut d = format!("{}/{} {}", i + 1, self.steps.len(), s.name());
        if !s.phase.is_empty() {
            d += &format!("  {}", s.phase);
        }
        d
    }

    /// Queue a move. Returns what to tell the performer.
    pub fn ask(&mut self, m: Move, bar: usize) -> String {
        let from = self.queued.unwrap_or(self.current);
        let last = self.steps.len() - 1;
        let to = match m {
            Move::Next if from >= last => return String::from("already on the last step"),
            Move::Prev if from == 0 => return String::from("already on the first step"),
            Move::Next => from + 1,
            Move::Prev => from - 1,
            Move::To(n) if n == 0 || n > self.steps.len() => {
                return format!("no step {}: the set has {}", n, self.steps.len())
            }
            Move::To(n) => n - 1,
        };
        if to == self.current && self.queued.is_some() {
            self.queued = None;
            self.built_on = None;
            return format!("stays on {}", self.describe(to));
        }
        self.queued = Some(to);
        self.built_on = None;
        format!("next: {}, on bar {}", self.describe(to), self.next_line(bar) + 1)
    }

    /// The bar the next phrase starts on, counting from 0.
    fn next_line(&self, bar: usize) -> usize {
        (bar / self.phrase + 1) * self.phrase
    }

    /// Bars until the queued step lands, for the status line.
    pub fn waiting(&self, bar: usize) -> Option<(usize, usize)> {
        let q = self.queued.or(self.held.as_ref().map(|h| h.1)).or(self.in_flight.map(|f| f.0))?;
        Some((q, self.next_line(bar) - bar))
    }

    /// Called every tick with the bar the player is in, the tempo playing,
    /// the generation it runs and the seconds since the session started.
    /// Returns plans to send, and lines to print.
    pub fn tick(
        &mut self,
        bar: usize,
        tempo: f32,
        generation: Generation,
        now: f32,
        planner: &mut LivePlanner,
    ) -> (Vec<Plan>, Vec<String>) {
        let (mut plans, mut said) = (Vec::new(), Vec::new());

        // A values-only step waiting for its line.
        if let Some((_, _, _, _)) = &self.held {
            if bar.is_multiple_of(self.phrase) && self.built_on != Some(bar) {
                let (plan, step, from, to) = self.held.take().expect("held");
                plans.push(plan);
                self.current = step;
                said.push(format!("now {}", self.describe(step)));
                if from != to && self.ramp_bars > 0.0 {
                    // The edit set the new tempo; put the old one back in the
                    // same breath and walk there.
                    plans.push(Plan::Control { base: generation, op: FastOp::Tempo(from) });
                    self.ramp = Some(self.new_ramp(from, to, generation, now));
                }
            }
        }

        // A queued step, built in the last bar of the phrase.
        if let Some(step) = self.queued {
            let last_bar = (bar + 1).is_multiple_of(self.phrase);
            if (last_bar || self.phrase == 1) && self.built_on != Some(bar) {
                self.built_on = Some(bar);
                self.queued = None;
                match Source::load(&self.steps[step].path) {
                    Err(e) => said.push(format!("{}: {}", self.describe(step), e)),
                    Ok(src) => match planner.plan(&src.text, generation) {
                        Err(err) => {
                            src.print_errors(&err);
                            said.push(format!("{} does not compile; staying", self.describe(step)));
                        }
                        Ok(Plan::Unchanged) => {
                            self.current = step;
                            said.push(format!("now {} (same as before)", self.describe(step)));
                        }
                        Ok(mut plan @ Plan::Swap { .. }) => {
                            let mut to = tempo;
                            if let Plan::Swap { engine, blend_bars, .. } = &mut plan {
                                *blend_bars = self.steps[step].blend.unwrap_or(self.blend_bars);
                                to = engine.tempo();
                                if to != tempo && self.ramp_bars > 0.0 {
                                    engine.set_tempo(tempo);
                                }
                            }
                            plans.push(plan);
                            self.in_flight = Some((step, tempo, to));
                        }
                        Ok(plan) => {
                            // Values only: it would apply the moment it is
                            // sent, so it waits for the line. Its tempo is
                            // read from the text.
                            let to = src_tempo(&src.text).unwrap_or(tempo);
                            self.held = Some((plan, step, tempo, to));
                        }
                    },
                }
            }
        }

        if let Some(r) = &self.ramp {
            let t = ((now - r.start) / r.secs).clamp(0.0, 1.0);
            let bpm = r.from + (r.to - r.from) * t;
            plans.push(Plan::Control { base: r.generation, op: FastOp::Tempo(bpm) });
            if t >= 1.0 {
                self.ramp = None;
            }
        }
        (plans, said)
    }

    /// The player swapped in a new engine. Returns what to print, if it was
    /// a step landing.
    pub fn landed(&mut self, generation: Generation, now: f32) -> Option<String> {
        let (step, from, to) = self.in_flight.take()?;
        self.current = step;
        if from != to && self.ramp_bars > 0.0 {
            self.ramp = Some(self.new_ramp(from, to, generation, now));
        }
        Some(format!("now {}", self.describe(step)))
    }

    fn new_ramp(&self, from: f32, to: f32, generation: Generation, now: f32) -> Ramp {
        let bars = self.ramp_bars.max(0.0);
        // Seconds for `bars` bars at the tempo halfway through the ramp.
        let secs = bars * 4.0 * 60.0 / ((from + to) / 2.0);
        Ramp { from, to, start: now, secs: secs.max(0.01), generation }
    }
}

/// The `tempo` line of a step, for a step that is applied as a value edit.
fn src_tempo(text: &str) -> Option<f32> {
    text.lines().find_map(|l| l.trim().strip_prefix("tempo ")?.split_whitespace().next()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatum_core::live::LivePlayer;
    use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

    const BASE: &str = r#"
tempo 120
module beats kit { kick_level 100% }
module bass acid { cutoff 800hz }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern line { 1.2 - 1.2 - 5.1 - 1.2 - }
track drums { play beat using kit out > master }
track bass  { play line using acid out > master }
"#;

    /// A set in a fresh directory: the base, the base with only a value
    /// changed, and the base faster with a track more.
    fn set_dir() -> std::path::PathBuf {
        // One directory per call: the tests run in parallel and each removes
        // its own when it is done.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tatum-setnav-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("01.synth"), BASE).unwrap();
        std::fs::write(dir.join("02.synth"), BASE.replace("cutoff 800hz", "cutoff 2khz")).unwrap();
        let faster =
            BASE.replace("tempo 120", "tempo 128") + "track echo { play line using acid level 0.3 out > master }\n";
        std::fs::write(dir.join("03.synth"), faster).unwrap();
        dir
    }

    /// Plays the set, calling `at_bar` once at the start of each bar with
    /// the navigation; returns, per bar, the step playing and the tempo.
    fn play(bars: usize, mut at_bar: impl FnMut(usize, &mut SetNav)) -> Vec<(usize, f32)> {
        let dir = set_dir();
        let steps = crate::set::load(&dir, 32).unwrap();
        let mut nav = SetNav::new(steps, 4, 2.0);
        let mut planner = LivePlanner::new();
        let mut player = LivePlayer::new();
        let first = Source::load(&nav.path()).unwrap();
        player.apply(planner.plan(&first.text, player.generation()).unwrap());
        player.start();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut seen = Vec::new();
        let mut rendered = 0usize;
        while seen.len() < bars {
            let now = rendered as f32 / SAMPLE_RATE;
            let e = player.engine().unwrap();
            let (bar, tempo) = (e.current_bar(), e.tempo());
            let first_block = bar == seen.len();
            if first_block {
                at_bar(bar, &mut nav);
            }
            let (plans, _) = nav.tick(bar, tempo, player.generation(), now, &mut planner);
            for p in plans {
                player.apply(p);
            }
            if first_block {
                seen.push((nav.current, player.engine().unwrap().tempo()));
            }
            if player.process(&mut l, &mut r).is_some() {
                nav.landed(player.generation(), now);
            }
            while player.take_retired().is_some() {}
            rendered += BLOCK_SIZE;
        }
        let _ = std::fs::remove_dir_all(&dir);
        seen
    }

    #[test]
    fn a_step_asked_for_mid_phrase_lands_on_the_next_phrase_line() {
        let seen = play(14, |bar, nav| {
            if bar == 1 || bar == 5 {
                nav.ask(Move::Next, bar);
            }
        });
        let steps: Vec<usize> = seen.iter().map(|s| s.0).collect();
        // Asked in bar 2 and bar 6 (counting from 1): in on bars 5 and 9.
        assert_eq!(&steps[..4], &[0, 0, 0, 0]);
        assert_eq!(&steps[4..8], &[1, 1, 1, 1], "{steps:?}");
        assert_eq!(steps[8], 2, "{steps:?}");
    }

    #[test]
    fn a_faster_step_ramps_from_the_tempo_that_was_playing() {
        let seen = play(14, |bar, nav| {
            if bar == 1 {
                nav.ask(Move::To(3), bar);
            }
        });
        let tempos: Vec<f32> = seen.iter().map(|s| s.1).collect();
        // Lands on bar 5 at 120, not 128, and is at 128 two bars later.
        assert_eq!(seen[4].0, 2);
        assert!((tempos[4] - 120.0).abs() < 1.0, "jumped: {tempos:?}");
        assert!(tempos[5] > 120.5 && tempos[5] < 127.5, "no ramp: {tempos:?}");
        assert!((tempos[8] - 128.0).abs() < 0.01, "{tempos:?}");
    }

    #[test]
    fn asking_for_the_step_that_plays_cancels_the_move() {
        let seen = play(9, |bar, nav| {
            if bar == 1 {
                nav.ask(Move::Next, bar);
            }
            if bar == 2 {
                nav.ask(Move::Prev, bar);
            }
        });
        assert!(seen.iter().all(|s| s.0 == 0), "{seen:?}");
    }
}

/// Not a check: a scripted performance rendered to a WAV, to hear the live
/// controls without a controller. `cargo test -p tatum-cli --release
/// performance_demo -- --ignored`; the file lands in `TATUM_DEMO_OUT`, and
/// `TATUM_DEMO_BLEND=8` blends the change of step over 8 bars.
#[cfg(test)]
mod demo {
    use super::*;
    use tatum_core::live::LivePlayer;
    use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

    const MIDI_DUB: &str = "\nmidi {\n  cc 70 > master dj cutoff 150hz..20khz..20khz\n  cc 70 > master hp cutoff 20hz..20hz..3khz\n  pad 44 > mute beat\n  pad 46 > throw stab\n  pad 47 > freeze\n}\n";
    const MIDI_DETROIT: &str = "\nmidi {\n  cc 70 > master dj cutoff 150hz..20khz..20khz\n  cc 70 > master hp cutoff 20hz..20hz..3khz\n  pad 44 > mute drums\n  pad 46 > throw stabs\n  pad 47 > freeze\n}\n";

    enum Act {
        Knob(u8, u8),
        Pad(u8, u8),
        Next,
    }

    /// A knob swept from `from` to `to` between two bars.
    fn sweep(out: &mut Vec<(f32, Act)>, cc: u8, bars: (f32, f32), from: u8, to: u8) {
        let n = ((bars.1 - bars.0) * 16.0) as usize;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let v = (from as f32 + (to as f32 - from as f32) * t).round() as u8;
            out.push((bars.0 + (bars.1 - bars.0) * t, Act::Knob(cc, v)));
        }
    }

    #[test]
    #[ignore]
    fn performance_demo() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let dir = std::env::temp_dir().join("tatum-demo-set");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dub = std::fs::read_to_string(root.join("examples/dub_techno.synth")).unwrap().replace(
            "       > compressor(-8, ratio=1.6, attack=30, release=280, makeup=1db)\n       > out",
            "       > compressor(-8, ratio=1.6, attack=30, release=280, makeup=1db)\n       > lowpass(20khz, 0.1) as dj > highpass(20hz, 0.1) as hp\n       > out",
        ) + MIDI_DUB;
        let detroit = std::fs::read_to_string(root.join("examples/detroit.synth")).unwrap().replace(
            "       > compressor(-7db, ratio=2.2, attack=18ms, release=140ms, makeup=4.23db)\n       > out",
            "       > compressor(-7db, ratio=2.2, attack=18ms, release=140ms, makeup=4.23db)\n       > lowpass(20khz, 0.1) as dj > highpass(20hz, 0.1) as hp\n       > out",
        ) + MIDI_DETROIT;
        assert!(dub.contains("as dj") && detroit.contains("as dj"));
        std::fs::write(dir.join("01-dub.synth"), dub).unwrap();
        std::fs::write(dir.join("02-detroit.synth"), detroit).unwrap();

        // Bars counted from 0.
        let mut script: Vec<(f32, Act)> = Vec::new();
        sweep(&mut script, 70, (0.0, 0.1), 64, 64); // the DJ filter starts open
        sweep(&mut script, 70, (12.0, 14.0), 64, 6); // closing: darker and darker
        sweep(&mut script, 70, (14.0, 15.0), 6, 64); // open again
        sweep(&mut script, 70, (15.0, 15.75), 64, 118); // thin: the lows go
        sweep(&mut script, 70, (15.75, 16.0), 118, 64); // and back on the one
        script.push((18.0, Act::Pad(44, 110))); // kick out for a bar
        script.push((19.0, Act::Pad(44, 0)));
        script.push((28.5, Act::Pad(46, 110))); // the chord thrown into the echo
        script.push((29.0, Act::Pad(46, 0)));
        script.push((32.0, Act::Pad(47, 110))); // the reverb frozen for two bars
        script.push((34.0, Act::Pad(47, 0)));
        script.push((35.0, Act::Next)); // detroit, on bar 41, ramping 122 -> 130
        sweep(&mut script, 70, (44.0, 46.0), 64, 10);
        sweep(&mut script, 70, (46.0, 47.0), 10, 64);
        script.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Long enough for an 8-bar blend to finish and be heard done.
        let end_bar = 56.0;

        let steps = crate::set::load(&dir, 32).unwrap();
        let mut nav = SetNav::new(steps, 8, 4.0);
        nav.blend_bars = std::env::var("TATUM_DEMO_BLEND").ok().and_then(|b| b.parse().ok()).unwrap_or(0.0);
        let mut planner = LivePlanner::new();
        let mut player = LivePlayer::new();
        let first = Source::load(&nav.path()).unwrap();
        player.apply(planner.plan(&first.text, player.generation()).unwrap());
        player.start();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
        let mut next = 0;
        loop {
            let e = player.engine().unwrap();
            let bar_f = e.global_step() as f32 / e.steps_per_bar() as f32;
            if bar_f >= end_bar {
                break;
            }
            let (bar, tempo, g) = (e.current_bar(), e.tempo(), player.generation());
            let now = out_l.len() as f32 / SAMPLE_RATE;
            while next < script.len() && script[next].0 <= bar_f {
                match script[next].1 {
                    Act::Knob(cc, v) => {
                        for p in planner.knob(cc, v, g).plans {
                            player.apply(p);
                        }
                    }
                    Act::Pad(note, v) => {
                        for p in planner.pad(note, v, g).unwrap_or_default() {
                            player.apply(p);
                        }
                    }
                    Act::Next => {
                        eprintln!("{}", nav.ask(Move::Next, bar));
                    }
                }
                next += 1;
            }
            let (plans, said) = nav.tick(bar, tempo, g, now, &mut planner);
            for p in plans {
                player.apply(p);
            }
            for s in said {
                eprintln!("bar {}: {}", bar + 1, s);
            }
            if player.process(&mut l, &mut r).is_some() {
                if let Some(s) = nav.landed(player.generation(), now) {
                    eprintln!("bar {}: {}", player.engine().unwrap().current_bar() + 1, s);
                }
            }
            while player.take_retired().is_some() {}
            out_l.extend_from_slice(&l);
            out_r.extend_from_slice(&r);
        }
        let out = std::env::var("TATUM_DEMO_OUT").unwrap_or_else(|_| "/tmp/tatum-actuacion.wav".into());
        std::fs::write(&out, tatum_core::wav::encode_stereo_16(&out_l, &out_r, SAMPLE_RATE as u32)).unwrap();
        eprintln!("wrote {} ({:.0} s)", out, out_l.len() as f32 / SAMPLE_RATE);
    }
}
