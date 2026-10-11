//! Performance effects on the whole output, for as long as a pad or a key
//! is held: a tape stop, a trance gate, a bitcrusher, a formant scream, a
//! filter cut and a rising sweep. Each comes in and goes out over a few
//! milliseconds, so pressing and letting go never clicks, and each takes an
//! amount, 0..1, from how hard the pad was struck and how hard it is pressed.
//!
//! Everything is allocated when the player is made: the audio thread only
//! ever processes.

use alloc::vec;
use alloc::vec::Vec;

use crate::effects::bitcrusher::Bitcrusher;
use crate::effects::formant::Formant;
use crate::math;
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::SAMPLE_RATE;

/// Which effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutFx {
    /// The output slows to a stop, like a turntable switched off.
    TapeStop,
    /// The output chopped on the grid: open for the first half of each
    /// division of this many sixteenths.
    Gate(f32),
    /// Fewer bits and a lower rate.
    Crush,
    /// A vowel filter driven hard: the output screams.
    Scream,
    /// A low-pass closing on the output.
    Cut,
    /// A high-pass climbing for as long as it is held.
    Sweep,
}

/// How far an effect's mix moves in a sample: in and out over 8 ms.
const FADE: f32 = 1.0 / (0.008 * SAMPLE_RATE);

/// The longest a tape stop runs before silence, and so how much it has to
/// keep: at a linear slowdown the read head falls half that behind.
const TAPE_MAX_SECONDS: f32 = 2.0;
const TAPE_LEN: usize = (TAPE_MAX_SECONDS * SAMPLE_RATE) as usize;

/// A mix that follows its target over `FADE`, and the amount it was given.
#[derive(Clone, Copy)]
struct Slot {
    on: bool,
    mix: f32,
    amount: f32,
}

impl Slot {
    fn new() -> Self {
        Self { on: false, mix: 0.0, amount: 0.0 }
    }

    fn active(&self) -> bool {
        self.on || self.mix > 0.0
    }

    #[inline]
    fn step(&mut self) -> f32 {
        let target = if self.on { 1.0 } else { 0.0 };
        if self.mix != target {
            let d = target - self.mix;
            self.mix = if math::abs(d) <= FADE { target } else { self.mix + FADE * d.signum() };
        }
        self.mix
    }
}

struct Tape {
    l: Vec<f32>,
    r: Vec<f32>,
    write: usize,
    read: f32,
    speed: f32,
}

impl Tape {
    fn read(buf: &[f32], at: f32) -> f32 {
        let n = buf.len();
        let i = math::floor(at) as usize % n;
        let f = at - math::floor(at);
        buf[i] * (1.0 - f) + buf[(i + 1) % n] * f
    }
}

struct Gate {
    division: f32,
    /// Samples into the division.
    pos: f32,
    level: f32,
}

struct Sweep {
    cutoff: f32,
    l: BiquadFilter,
    r: BiquadFilter,
}

/// Every performance effect, in the order the signal meets them.
pub struct PadFx {
    tape_slot: Slot,
    tape: Tape,
    gate_slot: Slot,
    gate: Gate,
    crush_slot: Slot,
    crush: Bitcrusher,
    scream_slot: Slot,
    scream: Formant,
    cut_slot: Slot,
    cut: Sweep,
    sweep_slot: Slot,
    sweep: Sweep,
    tick: u32,
}

impl Default for PadFx {
    fn default() -> Self {
        Self::new()
    }
}

impl PadFx {
    pub fn new() -> Self {
        let filter = |kind| {
            let mut f = BiquadFilter::new(SAMPLE_RATE);
            f.set_params(kind, if kind == FilterType::LowPass { 18000.0 } else { 30.0 }, 0.15);
            f
        };
        let mut scream = Formant::new(0, 0, 0.25, 0.0, 1.0);
        scream.set_position(0.25);
        Self {
            tape_slot: Slot::new(),
            tape: Tape { l: vec![0.0; TAPE_LEN], r: vec![0.0; TAPE_LEN], write: 0, read: 0.0, speed: 1.0 },
            gate_slot: Slot::new(),
            gate: Gate { division: 1.0, pos: 0.0, level: 1.0 },
            crush_slot: Slot::new(),
            crush: Bitcrusher::new(),
            scream_slot: Slot::new(),
            scream,
            cut_slot: Slot::new(),
            cut: Sweep { cutoff: 18000.0, l: filter(FilterType::LowPass), r: filter(FilterType::LowPass) },
            sweep_slot: Slot::new(),
            sweep: Sweep { cutoff: 30.0, l: filter(FilterType::HighPass), r: filter(FilterType::HighPass) },
            tick: 0,
        }
    }

    fn slot(&mut self, fx: OutFx) -> &mut Slot {
        match fx {
            OutFx::TapeStop => &mut self.tape_slot,
            OutFx::Gate(_) => &mut self.gate_slot,
            OutFx::Crush => &mut self.crush_slot,
            OutFx::Scream => &mut self.scream_slot,
            OutFx::Cut => &mut self.cut_slot,
            OutFx::Sweep => &mut self.sweep_slot,
        }
    }

    /// A pad down (`on`) or up, struck at `amount`.
    pub fn set(&mut self, fx: OutFx, on: bool, amount: f32) {
        let starting = on && !self.slot(fx).active();
        match fx {
            // A stop starts from the output as it is now.
            OutFx::TapeStop if starting => {
                self.tape.read = self.tape.write as f32;
                self.tape.speed = 1.0;
            }
            OutFx::Gate(d) => self.gate.division = d.max(0.5),
            OutFx::Sweep if starting => self.sweep.cutoff = 30.0,
            _ => {}
        }
        let slot = self.slot(fx);
        slot.on = on;
        if on {
            slot.amount = math::clamp(amount, 0.0, 1.0);
        }
    }

    /// How hard a held pad is pressed now.
    pub fn amount(&mut self, fx: OutFx, amount: f32) {
        let slot = self.slot(fx);
        if slot.on {
            slot.amount = math::clamp(amount, 0.0, 1.0);
        }
    }

    /// Whether anything is on or still fading out.
    pub fn active(&self) -> bool {
        [self.tape_slot, self.gate_slot, self.crush_slot, self.scream_slot, self.cut_slot, self.sweep_slot]
            .iter()
            .any(|s| s.active())
    }

    pub fn reset(&mut self) {
        for s in [
            &mut self.tape_slot,
            &mut self.gate_slot,
            &mut self.crush_slot,
            &mut self.scream_slot,
            &mut self.cut_slot,
            &mut self.sweep_slot,
        ] {
            *s = Slot::new();
        }
    }

    /// Process a block. `step` is a sixteenth in samples and `into_step`
    /// how far the block starts into the one playing, so the gate keeps to
    /// the grid.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32], step: f32, into_step: f32) {
        if !self.active() {
            return;
        }
        let division = self.gate.division * step.max(1.0);
        // The gate finds the grid again at every block.
        self.gate.pos = into_step % division.max(1.0);
        for i in 0..out_l.len().min(out_r.len()) {
            let (mut l, mut r) = (out_l[i], out_r[i]);
            let retune = self.tick.is_multiple_of(32);
            self.tick = self.tick.wrapping_add(1);

            // Sweep: a high-pass climbing toward 4 kHz while held, falling
            // back fast when let go.
            if self.sweep_slot.active() {
                let mix = self.sweep_slot.step();
                if retune {
                    let (target, secs) =
                        if self.sweep_slot.on { (4000.0, 4.0 - 3.0 * self.sweep_slot.amount) } else { (30.0, 0.12) };
                    let k = 32.0 / (secs * SAMPLE_RATE);
                    self.sweep.cutoff *= math::pow(target / self.sweep.cutoff, k.min(1.0));
                    self.sweep.l.set_cutoff(self.sweep.cutoff);
                    self.sweep.r.set_cutoff(self.sweep.cutoff);
                }
                let (fl, fr) = (self.sweep.l.process(l), self.sweep.r.process(r));
                l += (fl - l) * mix;
                r += (fr - r) * mix;
            }
            // Cut: a low-pass closing to 150..900 Hz by how hard it is held.
            if self.cut_slot.active() {
                let mix = self.cut_slot.step();
                if retune {
                    let target = if self.cut_slot.on { 900.0 - 750.0 * self.cut_slot.amount } else { 18000.0 };
                    let k = 32.0 / (0.06 * SAMPLE_RATE);
                    self.cut.cutoff *= math::pow(target / self.cut.cutoff, k.min(1.0));
                    self.cut.l.set_cutoff(self.cut.cutoff);
                    self.cut.r.set_cutoff(self.cut.cutoff);
                }
                let (fl, fr) = (self.cut.l.process(l), self.cut.r.process(r));
                l += (fl - l) * mix;
                r += (fr - r) * mix;
            }
            // Scream: driven into a vowel filter, a toward i by the pressure.
            if self.scream_slot.active() {
                let mix = self.scream_slot.step();
                if retune {
                    self.scream.set_position(0.15 + 0.45 * self.scream_slot.amount);
                }
                let drive = 2.0 + 4.0 * self.scream_slot.amount;
                let (sl, sr) = self.scream.process_stereo(math::tanh(l * drive), math::tanh(r * drive));
                l += (sl * 0.6 - l) * mix;
                r += (sr * 0.6 - r) * mix;
            }
            // Crush: 12 bits down to 3, the rate down with them.
            if self.crush_slot.active() {
                let mix = self.crush_slot.step();
                if retune {
                    let a = self.crush_slot.amount;
                    self.crush.set_bit_depth(12.0 - 9.0 * a);
                    self.crush.set_rate_reduce(0.1 + 0.55 * a);
                }
                let (cl, cr) = self.crush.process_stereo(l, r);
                l += (cl - l) * mix;
                r += (cr - r) * mix;
            }
            // Gate: open for the first half of each division; how far it
            // closes in the second half goes with how hard it is held.
            if self.gate_slot.active() {
                let mix = self.gate_slot.step();
                let open = self.gate.pos < division * 0.5;
                let floor = 0.4 - 0.4 * self.gate_slot.amount;
                let target = if open { 1.0 } else { floor };
                // 2 ms edges: square enough to chop, round enough not to click.
                let k = 1.0 / (0.002 * SAMPLE_RATE);
                self.gate.level += (target - self.gate.level) * k.min(1.0);
                self.gate.pos += 1.0;
                if self.gate.pos >= division {
                    self.gate.pos -= division;
                }
                let g = 1.0 + (self.gate.level - 1.0) * mix;
                l *= g;
                r *= g;
            }
            // Tape stop: the output read back slower and slower.
            if self.tape_slot.active() {
                let mix = self.tape_slot.step();
                let t = &mut self.tape;
                t.l[t.write] = l;
                t.r[t.write] = r;
                let (tl, tr) = if t.speed > 0.0 {
                    let tl = Tape::read(&t.l, t.read);
                    let tr = Tape::read(&t.r, t.read);
                    // Fades as it slows, so the stop ends in silence, not on
                    // a held sample.
                    let fade = math::clamp(t.speed * 4.0, 0.0, 1.0);
                    (tl * fade, tr * fade)
                } else {
                    (0.0, 0.0)
                };
                if self.tape_slot.on {
                    let secs = 1.6 - 1.3 * self.tape_slot.amount;
                    t.speed = (t.speed - 1.0 / (secs * SAMPLE_RATE)).max(0.0);
                    t.read += t.speed;
                    if t.read >= TAPE_LEN as f32 {
                        t.read -= TAPE_LEN as f32;
                    }
                }
                t.write = (t.write + 1) % TAPE_LEN;
                l += (tl - l) * mix;
                r += (tr - r) * mix;
            }
            out_l[i] = l;
            out_r[i] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> (Vec<f32>, Vec<f32>) {
        let s: Vec<f32> =
            (0..n).map(|i| (i as f32 * 330.0 / SAMPLE_RATE * core::f32::consts::TAU).sin() * 0.5).collect();
        (s.clone(), s)
    }

    const STEP: f32 = SAMPLE_RATE * 60.0 / 120.0 / 4.0;

    /// Block `k` of 128, and how far into its sixteenth it starts, as the
    /// player tells the gate.
    fn into(k: usize) -> f32 {
        (k * 128) as f32 % STEP
    }

    fn run(fx: &mut PadFx, l: &mut [f32], r: &mut [f32]) {
        for (k, (cl, cr)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
            fx.process(cl, cr, STEP, into(k));
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn off_it_leaves_the_output_alone() {
        let mut fx = PadFx::new();
        let (mut l, mut r) = tone(4096);
        let before = l.clone();
        run(&mut fx, &mut l, &mut r);
        assert_eq!(l, before);
    }

    #[test]
    fn every_effect_changes_the_sound_and_none_clicks() {
        for fx_kind in [OutFx::TapeStop, OutFx::Gate(1.0), OutFx::Crush, OutFx::Scream, OutFx::Cut, OutFx::Sweep] {
            let mut fx = PadFx::new();
            let (mut l, mut r) = tone(88200);
            let dry = l.clone();
            let mut out = Vec::new();
            for (k, (cl, cr)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
                if k == 40 {
                    fx.set(fx_kind, true, 1.0);
                }
                if k == 500 {
                    fx.set(fx_kind, false, 0.0);
                }
                fx.process(cl, cr, STEP, into(k));
                out.extend_from_slice(cl);
            }
            let held = &out[128 * 200..128 * 480];
            let diff: Vec<f32> = held.iter().zip(&dry[128 * 200..128 * 480]).map(|(a, b)| a - b).collect();
            assert!(rms(&diff) > 0.02, "{fx_kind:?} did nothing ({})", rms(&diff));
            // Let go, it is gone: the tail is the dry signal again.
            let tail = &out[128 * 600..];
            let back: f32 = tail.iter().zip(&dry[128 * 600..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(back < 0.05, "{fx_kind:?} still on after letting go ({back})");
            // In and out without a jump bigger than the tone itself makes
            // (a 330 Hz sine at 0.5 moves at most ~0.024 a sample).
            // A crusher is all steps by design: its edges are held to the
            // steps it makes while held, not to the sine's.
            let jump = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            let allowed = if fx_kind == OutFx::Crush { jump(held) + 0.05 } else { 0.25 };
            for edge in [128 * 40, 128 * 500] {
                let worst = jump(&out[edge - 64..edge + 512]);
                assert!(worst < allowed, "{fx_kind:?} clicks at {edge}: {worst} (allowed {allowed})");
            }
        }
    }

    #[test]
    fn a_tape_stop_ends_in_silence_sooner_when_struck_harder() {
        let silent_after = |amount: f32| {
            let mut fx = PadFx::new();
            fx.set(OutFx::TapeStop, true, amount);
            let (mut l, mut r) = tone(88200);
            run(&mut fx, &mut l, &mut r);
            l.chunks(441).position(|c| rms(c) < 1e-3).unwrap_or(usize::MAX)
        };
        let soft = silent_after(0.0);
        let hard = silent_after(1.0);
        assert!(hard < soft, "hard {hard} soft {soft}");
        assert!(hard < 40, "a hard stop takes {hard} hundredths of a second");
    }

    #[test]
    fn the_gate_keeps_to_the_grid() {
        let mut fx = PadFx::new();
        fx.set(OutFx::Gate(1.0), true, 1.0);
        let n = 88200;
        let (mut l, mut r) = (vec![0.5f32; n], vec![0.5f32; n]);
        run(&mut fx, &mut l, &mut r);
        let step = STEP as usize;
        // Well after the fade in: open in the first half of a sixteenth,
        // shut in the second.
        let k = 10 * step;
        assert!(l[k + step / 4] > 0.45, "{}", l[k + step / 4]);
        assert!(l[k + 3 * step / 4] < 0.05, "{}", l[k + 3 * step / 4]);
    }
}
