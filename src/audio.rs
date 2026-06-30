//! cpal audio output for the Photon notification synthesizer.
//!
//! `play(samples)` takes a mono `f32` buffer (as produced by `synth::PhotonVoice::render`)
//! and plays it once on the default output device, resampled trivially to the device's
//! native rate and fanned out to however many channels it wants. Playback runs on
//! cpal's own audio thread; `play` blocks the calling thread until the buffer is drained
//! so the synth → speaker path is a single synchronous call from the UI hotkey.
//!
//! Errors are deliberately swallowed into a `Result` with a human string rather than
//! panicking — a missing audio device should never take the plotter window down.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::synth::SAMPLE_RATE;

/// Play a mono `f32` buffer once, blocking until it finishes (or a safety timeout).
/// `samples` are assumed to be at [`SAMPLE_RATE`]; if the device runs at a different
/// rate we nearest-neighbour resample on the fly — fine for a sub-second notification.
pub fn play(samples: Vec<f32>) -> Result<(), String> {
    if samples.is_empty() {
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default audio output device".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("no default output config: {e}"))?;

    let device_rate = config.sample_rate() as f32;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    // Step through the source buffer at this rate so playback comes out at the right
    // pitch regardless of the device's native sample rate.
    let src_step = SAMPLE_RATE as f32 / device_rate;

    let buf = Arc::new(samples);
    let pos = Arc::new(AtomicUsize::new(0)); // source index << 8 fixed-point
    let done = Arc::new(AtomicBool::new(false));

    let total_src = buf.len();
    // Expected wall-clock duration, plus margin, used as the blocking timeout.
    let expected = Duration::from_secs_f32(total_src as f32 / SAMPLE_RATE as f32);

    let err_fn = |e| eprintln!("audio stream error: {e}");

    // One generic writer closure, instantiated per sample format. `write` fills the
    // device buffer frame by frame, advancing a fixed-point cursor through the source.
    macro_rules! make_stream {
        ($t:ty, $convert:expr) => {{
            let buf = Arc::clone(&buf);
            let pos = Arc::clone(&pos);
            let done = Arc::clone(&done);
            let stream_config: cpal::StreamConfig = config.clone().into();
            device.build_output_stream(
                stream_config,
                move |out: &mut [$t], _: &cpal::OutputCallbackInfo| {
                    // Cursor is fixed-point: integer part indexes the source buffer.
                    for frame in out.chunks_mut(channels) {
                        let cur = pos.load(Ordering::Relaxed);
                        let src_i = cur >> 8;
                        let sample = if src_i < total_src {
                            buf[src_i]
                        } else {
                            done.store(true, Ordering::Relaxed);
                            0.0
                        };
                        let v: $t = $convert(sample);
                        for ch in frame.iter_mut() {
                            *ch = v;
                        }
                        // Advance cursor by src_step in 8.8 fixed point.
                        let inc = (src_step * 256.0) as usize;
                        pos.store(cur + inc.max(1), Ordering::Relaxed);
                    }
                },
                err_fn,
                None,
            )
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => make_stream!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => {
            make_stream!(i16, |s: f32| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        }
        cpal::SampleFormat::U16 => make_stream!(u16, |s: f32| {
            ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
        }),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("failed to build output stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start stream: {e}"))?;

    // Block until the callback signals it ran off the end of the buffer, with a hard
    // timeout so a stalled device can't hang the UI thread.
    let deadline = Instant::now() + expected + Duration::from_millis(250);
    while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // A touch of tail so the last frames clear the DAC before the stream drops.
    std::thread::sleep(Duration::from_millis(30));
    Ok(())
}
