pub const PI: f32 = core::f32::consts::PI;
pub const TWO_PI: f32 = core::f32::consts::TAU;
pub const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;
pub const LN2: f32 = core::f32::consts::LN_2;
pub const INV_LN2: f32 = core::f32::consts::LOG2_E;

#[inline]
pub fn abs(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

#[inline]
pub fn clamp(x: f32, min: f32, max: f32) -> f32 {
    if x < min {
        min
    } else if x > max {
        max
    } else {
        x
    }
}

#[inline]
pub fn floor(x: f32) -> f32 {
    let i = x as i32;
    let fi = i as f32;
    if x < fi {
        fi - 1.0
    } else {
        fi
    }
}

#[inline]
pub fn fmod(x: f32, y: f32) -> f32 {
    x - floor(x / y) * y
}

// --- Sin LUT: 1024-entry table with linear interpolation ---

const SIN_TABLE_SIZE: usize = 1024;

const fn sin_for_table(phase: f64) -> f64 {
    // Reduce phase from [0, 2π) to [0, π/2] using symmetry
    let pi = core::f64::consts::PI;
    let half_pi = pi / 2.0;

    let mut x = phase;
    let mut sign = 1.0_f64;

    if x > pi {
        sign = -1.0;
        x -= pi;
    }
    if x > half_pi {
        x = pi - x;
    }

    // 13th-order Taylor series in f64 for high-precision table
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;
    let x11 = x9 * x2;
    let x13 = x11 * x2;

    let result = x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362880.0 - x11 / 39916800.0 + x13 / 6227020800.0;

    sign * result
}

const fn generate_sin_table() -> [f32; SIN_TABLE_SIZE] {
    let two_pi = core::f64::consts::TAU;
    let mut table = [0.0_f32; SIN_TABLE_SIZE];
    let mut i = 0;
    while i < SIN_TABLE_SIZE {
        let phase = (i as f64) * two_pi / (SIN_TABLE_SIZE as f64);
        table[i] = sin_for_table(phase) as f32;
        i += 1;
    }
    table
}

static SIN_TABLE: [f32; SIN_TABLE_SIZE] = generate_sin_table();

/// Sine via 1024-entry LUT with linear interpolation.
pub fn sin(x: f32) -> f32 {
    let mut norm = fmod(x, TWO_PI);
    if norm < 0.0 {
        norm += TWO_PI;
    }
    let idx_f = norm * (SIN_TABLE_SIZE as f32 / TWO_PI);
    let idx = idx_f as usize;
    let frac = idx_f - idx as f32;
    let i0 = idx & (SIN_TABLE_SIZE - 1);
    let i1 = (idx + 1) & (SIN_TABLE_SIZE - 1);
    SIN_TABLE[i0] + (SIN_TABLE[i1] - SIN_TABLE[i0]) * frac
}

#[inline]
pub fn cos(x: f32) -> f32 {
    sin(x + HALF_PI)
}

/// Fast tanh using (7,6) Padé approximant.
pub fn tanh(x: f32) -> f32 {
    if x > 5.0 {
        return 1.0;
    }
    if x < -5.0 {
        return -1.0;
    }
    let x2 = x * x;
    let x4 = x2 * x2;
    let x6 = x4 * x2;
    let num = x * (135135.0 + x2 * 17325.0 + x4 * 378.0 + x6);
    let den = 135135.0 + x2 * 62370.0 + x4 * 3150.0 + x6 * 28.0;
    num / den
}

/// Fast exp approximation.
pub fn exp(x: f32) -> f32 {
    if x > 88.0 {
        return 3.4028235e38;
    }
    if x < -87.0 {
        return 0.0;
    }
    let t = x * INV_LN2;
    let k = floor(t);
    let f = t - k;

    // 2^f polynomial for f in [0, 1)
    let p = 1.0 + f * (core::f32::consts::LN_2 + f * (0.2402265 + f * (0.0555041 + f * 0.0096139)));

    // 2^k via bit manipulation
    let ki = k as i32;
    if ki < -126 {
        return 0.0;
    }
    if ki > 127 {
        return 3.4028235e38;
    }
    let bits = ((ki + 127) as u32) << 23;
    let pow2k = f32::from_bits(bits);

    p * pow2k
}

/// Fast square root using Newton's method with bit manipulation initial guess.
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let bits = x.to_bits();
    let guess_bits = (bits >> 1) + 0x1FBB_4000;
    let mut y = f32::from_bits(guess_bits);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// 2^x for arbitrary float x.
pub fn pow2(x: f32) -> f32 {
    exp(x * LN2)
}

/// Natural logarithm approximation.
pub fn ln(x: f32) -> f32 {
    if x <= 0.0 {
        return -3.4028235e38;
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xFF) as i32 - 127;
    let m_bits = (bits & 0x007F_FFFF) | 0x3F80_0000;
    let mut m = f32::from_bits(m_bits); // m in [1, 2)

    // The four-term Taylor series for ln(1+f) that used to live here is only good
    // near f = 0: at f close to 1 it was off by 0.11, which is 11% once it goes
    // through exp(). Every exponential mapping in the engine (filter cutoffs,
    // envelope times) inherited that error, so the documented anchors were wrong.
    //
    // Reduce the mantissa to [sqrt(1/2), sqrt(2)) and use ln(m) = 2*atanh(s) with
    // s = (m-1)/(m+1), which stays under 0.172 and converges in four terms.
    if m > core::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let ln_m = s * (2.0 + s2 * (0.6666667 + s2 * (0.4 + s2 * 0.2857143)));
    ln_m + (e as f32) * LN2
}

/// Base-10 logarithm.
#[inline]
pub fn log10(x: f32) -> f32 {
    ln(x) * core::f32::consts::LOG10_E
}

/// Power function: x^y = exp(y * ln(x))
pub fn pow(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    exp(y * ln(x))
}

// Semitone ratios for MIDI-to-frequency conversion (relative to C)
// approx_constant: F# is 2^(6/12), and the table is that formula to six decimals
// throughout; SQRT_2 is a different f32 and would detune one note out of twelve.
#[allow(clippy::approx_constant)]
const SEMITONE_RATIOS: [f32; 12] = [
    1.000000, // C
    1.059463, // C#
    1.122462, // D
    1.189207, // D#
    1.259921, // E
    1.33484,  // F
    1.414214, // F#
    1.498307, // G
    1.587401, // G#
    1.681793, // A
    1.781797, // A#
    1.887749, // B
];

/// Convert MIDI note number to frequency in Hz.
/// A4 (MIDI 69) = 440 Hz.
pub fn midi_to_freq(note: u8) -> f32 {
    let n = note as i32;
    // A4 = 440Hz is MIDI 69. A4 is in octave 5 (C5..B5 = MIDI 60..71)
    // Reference: C0 = MIDI 12, freq ≈ 16.3516 Hz
    // freq = 440 * 2^((note - 69) / 12)
    let diff = n - 69;
    let octaves = if diff >= 0 { diff / 12 } else { (diff - 11) / 12 };
    let semitones = (diff - octaves * 12) as usize;

    let mut freq = 440.0 * SEMITONE_RATIOS[semitones];

    if octaves > 0 {
        let mut i = 0;
        while i < octaves {
            freq *= 2.0;
            i += 1;
        }
    } else {
        let mut i = 0;
        while i < -octaves {
            freq *= 0.5;
            i += 1;
        }
    }

    freq
}

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Convert decibels to linear amplitude.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    pow(10.0, db * 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sin_accuracy() {
        let test_cases: [(f32, f32); 8] = [
            (0.0, 0.0),
            (HALF_PI, 1.0),
            (PI, 0.0),
            (-HALF_PI, -1.0),
            (PI / 6.0, 0.5),          // sin(30°)
            (PI / 4.0, 0.707_106_77), // sin(45°)
            (PI / 3.0, 0.866_025_4),  // sin(60°)
            (3.0 * PI, 0.0),          // sin(3π)
        ];
        for (x, expected) in test_cases {
            let result = sin(x);
            assert!(abs(result - expected) < 0.001, "sin({}) = {}, expected {}", x, result, expected);
        }
        // Negative input
        let r = sin(-PI);
        assert!(abs(r) < 0.001, "sin(-π) = {}, expected ~0", r);
        // Large input
        let r = sin(100.0);
        let expected = -0.506_365_66_f32; // sin(100) reference
        assert!(abs(r - expected) < 0.001, "sin(100) = {}, expected {}", r, expected);
    }

    #[test]
    fn test_sin_lut_coverage() {
        // Test 100 evenly-spaced points in [0, 2π]
        for i in 0..100 {
            let x = TWO_PI * (i as f32) / 100.0;
            let result = sin(x);
            // Reference via f64 Taylor (same method used for table, but at arbitrary phase)
            let xd = x as f64;
            let reference = {
                let pi = core::f64::consts::PI;
                let half_pi = pi / 2.0;
                let mut a = xd % (2.0 * pi);
                if a < 0.0 {
                    a += 2.0 * pi;
                }
                let mut s = 1.0_f64;
                if a > pi {
                    s = -1.0;
                    a -= pi;
                }
                if a > half_pi {
                    a = pi - a;
                }
                let a2 = a * a;
                let a3 = a2 * a;
                let a5 = a3 * a2;
                let a7 = a5 * a2;
                let a9 = a7 * a2;
                let a11 = a9 * a2;
                let a13 = a11 * a2;
                s * (a - a3 / 6.0 + a5 / 120.0 - a7 / 5040.0 + a9 / 362880.0 - a11 / 39916800.0 + a13 / 6227020800.0)
            } as f32;
            assert!(abs(result - reference) < 0.001, "sin({}) = {}, reference {}", x, result, reference);
        }
    }

    #[test]
    fn test_tanh_accuracy() {
        // Test points with f64 reference values
        let test_cases: [(f32, f64); 9] = [
            (0.0, 0.0),
            (0.5, 0.46211715726000974),
            (1.0, 0.7615941559557649),
            (1.5, 0.9051482536448664),
            (2.0, 0.9640275800758169),
            (2.5, 0.9866142981514303),
            (3.0, 0.9950547536867305),
            (-1.0, -0.7615941559557649),
            (-3.0, -0.9950547536867305),
        ];
        for (x, reference) in test_cases {
            let result = tanh(x) as f64;
            let ax = if x < 0.0 { -x } else { x };
            if ax <= 0.1 {
                // Absolute error for small values
                assert!((result - reference).abs() < 0.001, "tanh({}) = {}, reference {}", x, result, reference);
            } else {
                // Relative error for larger values
                let rel_err = ((result - reference) / reference).abs();
                assert!(
                    rel_err < 0.001,
                    "tanh({}) = {}, reference {}, rel_err = {:.6}%",
                    x,
                    result,
                    reference,
                    rel_err * 100.0
                );
            }
        }
    }

    #[test]
    fn test_midi_to_freq() {
        let a4 = midi_to_freq(69);
        assert!(abs(a4 - 440.0) < 0.01, "A4 should be 440Hz, got {}", a4);

        let a3 = midi_to_freq(57);
        assert!(abs(a3 - 220.0) < 0.5, "A3 should be 220Hz, got {}", a3);

        let a2 = midi_to_freq(45);
        assert!(abs(a2 - 110.0) < 0.5, "A2 should be 110Hz, got {}", a2);
    }

    #[test]
    fn test_exp_basic() {
        let e0 = exp(0.0);
        assert!(abs(e0 - 1.0) < 0.01, "exp(0) should be 1, got {}", e0);

        let e1 = exp(1.0);
        assert!(abs(e1 - core::f32::consts::E) < 0.05, "exp(1) should be ~2.718, got {}", e1);
    }

    #[test]
    fn test_sqrt_basic() {
        let s4 = sqrt(4.0);
        assert!(abs(s4 - 2.0) < 0.01, "sqrt(4) should be 2, got {}", s4);

        let s2 = sqrt(2.0);
        assert!(abs(s2 - core::f32::consts::SQRT_2) < 0.01, "sqrt(2) should be ~1.414, got {}", s2);
    }
}
