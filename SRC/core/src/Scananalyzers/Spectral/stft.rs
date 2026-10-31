//! Short-time Fourier transform: Hann-windowed magnitude frames, shared by the
//! frame-based extractors (spectral flux, MFCC).
use rustfft::{num_complex::Complex, FftPlanner};

/// Cap on the number of frames analyzed (long files use evenly spaced frames
/// from the start; at the default 2048/512 this covers ~35 s of 44.1 kHz audio).
const MAX_FRAMES: usize = 3000;

/// Compute Hann-windowed magnitude spectra (first `n_fft / 2` bins per frame).
/// A file shorter than one frame is zero-padded into a single frame.
pub fn stft_mags(data: &[f32], n_fft: usize, hop: usize) -> Vec<Vec<f32>> {
    if data.is_empty() || n_fft < 2 {
        return Vec::new();
    }
    let window: Vec<f32> = (0..n_fft)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n_fft as f32 - 1.0)).cos())
        .collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n_fft);

    let n_frames = if data.len() <= n_fft { 1 } else { 1 + (data.len() - n_fft) / hop }.min(MAX_FRAMES);
    let half = n_fft / 2;
    let mut frames = Vec::with_capacity(n_frames);
    let mut buf = vec![Complex { re: 0.0f32, im: 0.0f32 }; n_fft];
    for fi in 0..n_frames {
        let start = fi * hop;
        for i in 0..n_fft {
            let x = data.get(start + i).copied().unwrap_or(0.0);
            buf[i] = Complex { re: x * window[i], im: 0.0 };
        }
        fft.process(&mut buf);
        frames.push(buf[..half].iter().map(|c| c.norm()).collect());
    }
    frames
}
