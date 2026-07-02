//! Photon notification synthesizer.
//!
//! Produces a short, deterministic "you've got a Photon" notification sound. Every
//! user gets a *distinct* sound derived from a `u64` seed (their id / key), but all
//! seeds share a family resemblance — the same three-act structure and the same
//! frequency-band / sweep-shape envelope — so any Photon notification is recognizable
//! by texture even though no two are byte-identical.
//!
//! The aesthetic target is a 1990s dial-up modem handshake (FSK warble + rising
//! sweep) that resolves into a two-note bird chirp. The modem half says "a machine is
//! talking to you"; the chirp says "and it's friendly."
//!
//! **All oscillator math runs in Spirix `S43`**, exactly like the plotter — the same
//! `voice(t)` closure feeds both the audio buffer (`render`) and the plot curve, so
//! what you *see* is literally what you *hear*. Only the final clamp-to-`f32` at the
//! audio/pixel boundary leaves Scalar space.
//!
//! Determinism: no `rand`, no clock. Everything is a pure function of `seed`. The same
//! seed always yields the same waveform — that is the whole point (a user's sound is
//! their identity).

use spirix::ScalarF4E3 as S43;

/// Output sample rate. 44.1 kHz — standard, plenty for a sub-second chirp.
pub const SAMPLE_RATE: u32 = 44_100;

/// Total notification length in seconds. Short enough to be a notification, long
/// enough to carry a recognizable melody-shape.
pub const DURATION_SECS: f32 = 1.05;

/// The notification is three acts laid end to end. Fractions of the total duration.
const ACT_HANDSHAKE: f32 = 0.34; // FSK warble — "a modem is dialing"
const ACT_SWEEP: f32 = 0.30; // rising glide — "connecting…"
// remainder (0.36) is the bird chirp tail.

/// Style-envelope frequency bands (Hz). Per-seed parameters are drawn *within* these
/// bands so the family always lives in the same register — that shared register is a
/// big part of what makes them all sound like "Photon."
const WARBLE_LO_BAND: (f32, f32) = (620.0, 980.0);
const WARBLE_HI_BAND: (f32, f32) = (1180.0, 1720.0);
const SWEEP_START_BAND: (f32, f32) = (520.0, 760.0);
const SWEEP_END_BAND: (f32, f32) = (1600.0, 2300.0);
const CHIRP_BAND: (f32, f32) = (1900.0, 3200.0);

/// Per-seed voice parameters, all derived deterministically from the seed. Holding
/// them in one struct keeps `voice()` a pure function of `(params, t)` so it can be
/// shared verbatim between audio and plot.
#[derive(Clone, Copy, Debug)]
pub struct PhotonVoice {
    seed: u64,
    warble_lo: f32,
    warble_hi: f32,
    /// FSK toggle rate (Hz) — how fast the warble alternates between lo and hi tones.
    warble_rate: f32,
    sweep_start: f32,
    sweep_end: f32,
    chirp_a: f32,
    chirp_b: f32,
}

impl PhotonVoice {
    /// Derive a voice from a user seed. A tiny SplitMix64-style hash spreads seed bits
    /// across the parameters so adjacent ids (1, 2, 3…) still sound clearly different
    /// while staying inside the shared bands.
    pub fn from_seed(seed: u64) -> Self {
        let mut s = seed;
        let mut next = || {
            // SplitMix64 — well-mixed, deterministic, no deps.
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            // Map to [0, 1).
            ((z ^ (z >> 31)) >> 11) as f32 / (1u64 << 53) as f32
        };
        let lerp = |band: (f32, f32), u: f32| band.0 + (band.1 - band.0) * u;

        PhotonVoice {
            seed,
            warble_lo: lerp(WARBLE_LO_BAND, next()),
            warble_hi: lerp(WARBLE_HI_BAND, next()),
            warble_rate: 14.0 + 26.0 * next(), // 14–40 Hz toggle
            sweep_start: lerp(SWEEP_START_BAND, next()),
            sweep_end: lerp(SWEEP_END_BAND, next()),
            chirp_a: lerp(CHIRP_BAND, next()),
            chirp_b: lerp(CHIRP_BAND, next()),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The instantaneous waveform value at time `t` seconds, in `[-1, 1]`-ish range
    /// (act envelopes keep it bounded). **Runs entirely in S43** — this is the single
    /// source of truth fed to both the speaker and the plot.
    ///
    /// We accumulate *phase* per act rather than evaluating `sin(2π f t)` directly so
    /// the swept act has no phase discontinuity at its boundaries. For the constant /
    /// FSK acts a direct phase works fine and keeps the math legible.
    pub fn voice(&self, t: f32) -> S43 {
        let tau = S43::TAU;

        if t < ACT_HANDSHAKE {
            // --- Act 1: FSK modem warble ---
            // Square-wave toggle between lo and hi tone selects the carrier; a gentle
            // amplitude shimmer sells the "data" feel.
            let local = t / ACT_HANDSHAKE; // 0..1 within the act
            let toggle = (t * self.warble_rate).floor() as i64;
            let f = if toggle & 1 == 0 {
                self.warble_lo
            } else {
                self.warble_hi
            };
            let phase = tau * S43::from((f as f64 * t as f64) % 1.0);
            let carrier = phase.sin();
            // Slight second-harmonic grit for the metallic modem timbre.
            let grit = (phase + phase).sin() * S43::from(0.25);
            let env = act_window(local) * S43::from(0.55);
            (carrier + grit) * env
        } else if t < ACT_HANDSHAKE + ACT_SWEEP {
            // --- Act 2: rising connect sweep ---
            // Linear frequency glide start→end. Phase is the integral of a linear
            // frequency ramp: ∫₀ˣ (f0 + (f1−f0)·u/T) du.
            let x = t - ACT_HANDSHAKE; // seconds into the sweep
            let span = ACT_SWEEP;
            let f0 = self.sweep_start as f64;
            let f1 = self.sweep_end as f64;
            let xd = x as f64;
            let inst_phase = f0 * xd + (f1 - f0) * xd * xd / (2.0 * span as f64);
            let phase = tau * S43::from(inst_phase % 1.0);
            let local = x / span;
            phase.sin() * act_window(local) * S43::from(0.6)
        } else {
            // --- Act 3: two-note bird chirp ---
            // Two short descending glides ("chir-eep"), each with a fast attack and a
            // longer decay. The two notes are the chirp_a/chirp_b pair.
            let x = t - ACT_HANDSHAKE - ACT_SWEEP;
            let span = DURATION_SECS - ACT_HANDSHAKE - ACT_SWEEP;
            let half = span * 0.5;
            let (f_base, local) = if x < half {
                (self.chirp_a, x / half)
            } else {
                (self.chirp_b, (x - half) / half)
            };
            // Each note glides downward ~30% over its life — birdy, not robotic.
            let glide = f_base as f64 * (1.0 - 0.3 * local as f64);
            let xd = x as f64;
            let phase = tau * S43::from((glide * xd) % 1.0);
            // Pluck envelope: fast attack, exponential-ish decay (1−local)².
            let amp = (1.0 - local) * (1.0 - local);
            phase.sin() * S43::from(amp * 0.7)
        }
    }

    /// Render the whole notification into an `f32` sample buffer at `SAMPLE_RATE`.
    /// This is the audio boundary: S43 → f32, soft-clamped to `[-1, 1]`.
    pub fn render(&self) -> Vec<f32> {
        let n = (SAMPLE_RATE as f32 * DURATION_SECS) as usize;
        let mut out = Vec::with_capacity(n);
        let dt = 1.0 / SAMPLE_RATE as f32;
        for i in 0..n {
            let t = i as f32 * dt;
            let v = self.voice(t).to_f32();
            // A short global fade-in/out kills click artifacts at buffer ends.
            let fade = global_fade(i, n);
            out.push((v * fade).clamp(-1.0, 1.0));
        }
        out
    }
}

/// Sample a parsed formula across `[x_min, x_max]` and render it as an audio buffer — "play exactly what you plotted."
/// `x` sweeps the visible plot x-range linearly over `duration` seconds (so the curve you see IS the waveform you hear), and `y = f(x)` is the speaker displacement mapped through the visible y-range `[y_min, y_max]` to the audio's full scale `[-1, 1]`.
///
/// `eval` takes the world-x as f64 and returns f32: the caller owns the numeric type (the same precision-dispatched type the plot uses), so what you see IS what you hear.
/// Return NaN for undefined and ±∞ for escaped states.
///
/// The y-axis IS the volume, 1:1: `y_max` → +1.0, `y_min` → -1.0.
/// There is **no normalization** — if the curve runs off the top/bottom of the view it hard-clips at ±1.0 and audibly distorts, exactly as a cut-off waveform should.
/// Non-finite samples rail (±∞) or mute (NaN) rather than popping.
/// A short global fade at the buffer ends suppresses DAC clicks without touching the body.
pub fn render_formula<F>(
    eval: F,
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
    duration: f32,
) -> Vec<f32>
where
    F: Fn(f64) -> f32,
{
    let n = (SAMPLE_RATE as f32 * duration) as usize;
    if n == 0 || !(x_max > x_min) || !(y_max > y_min) {
        return Vec::new();
    }
    let x_span = x_max - x_min;
    let y_span = y_max - y_min;
    let n_f = n.max(1) as f32;
    let inv_os = 1.0 / OVERSAMPLE as f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // Stratified-jittered oversampling: each output sample averages OVERSAMPLE evaluations taken at RANDOM positions within its 1/SR window (one per sub-slice), rather than a fixed point.
        // Point-sampling a formula whose content runs past Nyquist folds it back as inharmonic tones + ringing; jittering decorrelates the samples so that aliasing becomes broadband noise instead (stochastic sampling), and the average knocks that noise down ~√OVERSAMPLE while preserving the de-aliasing (no sinc sidelobe ringing, unlike a fixed box filter).
        // Set OVERSAMPLE = 1 for the pure single-jitter version.
        let mut acc = 0.0f32;
        for k in 0..OVERSAMPLE {
            let sub = (k as f32 + sample_jitter(i, k)) * inv_os; // random spot in sub-slice k
            let frac = (i as f32 + sub) / n_f;
            let y = eval(x_min as f64 + frac as f64 * x_span as f64);
            // Map through the view; non-finite (∞ / escaped / undefined) rails or mutes, not pops.
            // Averaging a window that straddles a pole softens the spike instead of clicking.
            acc += if y.is_finite() {
                ((y - y_min) / y_span) * 2.0 - 1.0
            } else if y == f32::INFINITY {
                1.0
            } else if y == f32::NEG_INFINITY {
                -1.0
            } else {
                0.0
            };
        }
        let sample = (acc * inv_os).clamp(-1.0, 1.0);
        out.push(sample * global_fade(i, n));
    }
    out
}

/// Jittered sub-samples averaged per output sample (audio anti-aliasing).
/// `1` = a single random sample within each 1/SR window: cheapest, converts aliasing to noise with no averaging.
/// Raise it (e.g. 8) to trade evals for ~√OVERSAMPLE lower noise.
const OVERSAMPLE: usize = 1;

/// Deterministic per-(sample, sub-sample) jitter in `[0, 1)` — SplitMix64 hash.
/// Deterministic so a given formula always renders the same clip (reproducible), and well-mixed so the jitter reads as white noise rather than a periodic pattern.
#[inline]
fn sample_jitter(i: usize, k: usize) -> f32 {
    let mut h = (i as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((k as u64).wrapping_add(1).wrapping_mul(0xD1B5_4A32_D192_ED03));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    // Top 24 bits → f32 in [0, 1).
    (h >> 40) as f32 / (1u32 << 24) as f32
}

/// Per-act amplitude window: quick ramp up, sustain, quick ramp down. Keeps each act's
/// edges click-free without flattening the body. `local` in `[0, 1]`.
fn act_window(local: f32) -> S43 {
    const EDGE: f32 = 0.12;
    let a = if local < EDGE {
        local / EDGE
    } else if local > 1.0 - EDGE {
        (1.0 - local) / EDGE
    } else {
        1.0
    };
    S43::from(a.clamp(0.0, 1.0))
}

/// Global 4 ms fade at both ends of the whole buffer — final guard against DAC clicks.
fn global_fade(i: usize, n: usize) -> f32 {
    let edge = (SAMPLE_RATE as f32 * 0.004) as usize;
    if i < edge {
        i as f32 / edge as f32
    } else if i + edge >= n {
        (n - i) as f32 / edge as f32
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_seed() {
        let a = PhotonVoice::from_seed(0xCAFE).render();
        let b = PhotonVoice::from_seed(0xCAFE).render();
        assert_eq!(a, b, "same seed must produce identical audio");
    }

    #[test]
    fn distinct_seeds_differ() {
        let a = PhotonVoice::from_seed(1).render();
        let b = PhotonVoice::from_seed(2).render();
        assert_ne!(a, b, "adjacent seeds must sound different");
    }

    #[test]
    fn bounded_output() {
        let buf = PhotonVoice::from_seed(42).render();
        assert!(buf.iter().all(|s| s.abs() <= 1.0), "samples must stay in [-1, 1]");
        assert!(!buf.is_empty());
    }
}
