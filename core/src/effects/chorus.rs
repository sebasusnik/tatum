use crate::primitives::lfo::Lfo;
use crate::SAMPLE_RATE;

/// Simple chorus using a modulated delay line with cubic Hermite interpolation.
///
/// Stereo by way of one delay line per channel reading opposite sides of the
/// same LFO: when the left tap is long the right one is short. A single line
/// shared by both channels is not a chorus at all — with L == R it is a comb,
/// nulls every 40 Hz at a 25 ms tap, and it leaves the two channels identical.
pub struct Chorus {
    buffer: [f32; 4096],
    buffer_r: [f32; 4096],
    write_pos: usize,
    lfo: Lfo,
    base_delay: f32, // in samples (~25ms)
    depth: f32,      // in samples (~5ms)
    pub mix: f32,
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Chorus {
    pub fn new() -> Self {
        let mut lfo = Lfo::new(SAMPLE_RATE);
        lfo.set_rate(1.2);
        lfo.set_depth(1.0);
        Self {
            buffer: [0.0; 4096],
            buffer_r: [0.0; 4096],
            write_pos: 0,
            lfo,
            base_delay: 1100.0, // ~25ms at 44100
            depth: 220.0,       // ~5ms at 44100
            mix: 0.3,
        }
    }

    fn read_buffer(&self, offset: usize) -> f32 {
        Self::tap(&self.buffer, self.write_pos, offset)
    }

    #[inline]
    fn tap(buf: &[f32; 4096], write_pos: usize, offset: usize) -> f32 {
        if write_pos >= offset {
            buf[write_pos - offset]
        } else {
            buf[buf.len() - (offset - write_pos)]
        }
    }

    /// Cubic Hermite (Catmull-Rom) read of `buf` at a fractional delay.
    #[inline]
    fn read_interp(buf: &[f32; 4096], write_pos: usize, delay: f32) -> f32 {
        let delay_int = delay as usize;
        let delay_frac = delay - delay_int as f32;
        let s0 = Self::tap(buf, write_pos, delay_int + 1);
        let s1 = Self::tap(buf, write_pos, delay_int);
        let s2 = Self::tap(buf, write_pos, if delay_int > 0 { delay_int - 1 } else { 0 });
        let s3 = Self::tap(buf, write_pos, if delay_int > 1 { delay_int - 2 } else { 0 });
        let t = delay_frac;
        let t2 = t * t;
        let t3 = t2 * t;
        s1 + 0.5 * t * (s2 - s0)
            + t2 * (s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3)
            + t3 * (-0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3)
    }

    /// One LFO step per stereo frame; the right tap rides the opposite side of
    /// it, so the two channels decorrelate instead of combing together.
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.buffer[self.write_pos] = l;
        self.buffer_r[self.write_pos] = r;

        let lfo_val = self.lfo.next_sample();
        let dl = self.base_delay + lfo_val * self.depth;
        let dr = self.base_delay - lfo_val * self.depth;
        let wl = Self::read_interp(&self.buffer, self.write_pos, dl);
        let wr = Self::read_interp(&self.buffer_r, self.write_pos, dr);

        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        let dry = 1.0 - self.mix;
        (l * dry + wl * self.mix, r * dry + wr * self.mix)
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.buffer[self.write_pos] = input;

        let lfo_val = self.lfo.next_sample();
        let delay = self.base_delay + lfo_val * self.depth;
        let delay_int = delay as usize;
        let delay_frac = delay - delay_int as f32;

        // Cubic Hermite (Catmull-Rom) interpolation: 4 samples
        let s0 = self.read_buffer(delay_int + 1);
        let s1 = self.read_buffer(delay_int);
        let s2 = self.read_buffer(if delay_int > 0 { delay_int - 1 } else { 0 });
        let s3 = self.read_buffer(if delay_int > 1 { delay_int - 2 } else { 0 });

        let t = delay_frac;
        let t2 = t * t;
        let t3 = t2 * t;

        let delayed = s1
            + 0.5 * t * (s2 - s0)
            + t2 * (s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3)
            + t3 * (-0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3);

        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        input * (1.0 - self.mix) + delayed * self.mix
    }

    pub fn reset(&mut self) {
        self.buffer = [0.0; 4096];
        self.buffer_r = [0.0; 4096];
        self.write_pos = 0;
        self.lfo.reset();
    }
}
