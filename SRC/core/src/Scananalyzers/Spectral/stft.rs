//! Short-time Fourier transform: Hann-windowed magnitude frames, shared by the
//! frame-based extractors (spectral flux, MFCC, centroid stats, morphology).
use rustfft::{num_complex::Complex, FftPlanner};

/// Most frames kept from one file. At the default 2048/512 this is ~35 s of
/// 44.1 kHz audio; a longer file is not truncated but sampled evenly across its
/// whole length (see [`Stft`]), so memory stays bounded however long the input.
pub const MAX_FRAMES: usize = 3000;

/// The frames of one file, spaced `hop` samples apart.
///
/// A file of up to [`MAX_FRAMES`] analysis frames gets every frame and `hop` is
/// the analysis hop. A longer one keeps every `stride`-th frame, so the frames
/// still span the whole file and `hop` (= analysis hop × stride) is their real
/// spacing — frame *i* starts at sample *i · hop* either way. Because flux is a
/// frame-to-*next*-frame measure, a strided transform also keeps, for each kept
/// frame, the frame one analysis hop later in `next`; flux reads those pairs so
/// it measures the same 11.6 ms change on a long file as on a short one.
pub struct Stft {
    pub frames: Vec<Vec<f32>>,
    /// Spacing of `frames` in samples.
    pub hop: usize,
    /// Empty when every frame was kept; otherwise `next[i]` is the frame one
    /// analysis hop after `frames[i]` (zero-padded past the end).
    pub next: Vec<Vec<f32>>,
}

/// Compute Hann-windowed magnitude spectra (first `n_fft / 2` bins per frame).
/// A file shorter than one frame is zero-padded into a single frame.
pub fn stft(data: &[f32], n_fft: usize, hop: usize) -> Stft {
    if data.is_empty() || n_fft < 2 || hop == 0 {
        return Stft { frames: Vec::new(), hop, next: Vec::new() };
    }
    let window: Vec<f32> = (0..n_fft)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n_fft as f32 - 1.0)).cos())
        .collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n_fft);
    let half = n_fft / 2;
    let mut buf = vec![Complex { re: 0.0f32, im: 0.0f32 }; n_fft];
    let mut frame_at = |start: usize| -> Vec<f32> {
        for i in 0..n_fft {
            let x = data.get(start + i).copied().unwrap_or(0.0);
            buf[i] = Complex { re: x * window[i], im: 0.0 };
        }
        fft.process(&mut buf);
        buf[..half].iter().map(|c| c.norm()).collect()
    };

    let total = if data.len() <= n_fft { 1 } else { 1 + (data.len() - n_fft) / hop };
    let stride = total.div_ceil(MAX_FRAMES);
    let kept = total.div_ceil(stride);
    let mut frames = Vec::with_capacity(kept);
    let mut next = Vec::with_capacity(if stride > 1 { kept } else { 0 });
    for k in 0..kept {
        let start = k * stride * hop;
        frames.push(frame_at(start));
        if stride > 1 {
            next.push(frame_at(start + hop));
        }
    }
    Stft { frames, hop: hop * stride, next }
}

/// The frames alone, for callers that only need spectra (tests, short clips).
pub fn stft_mags(data: &[f32], n_fft: usize, hop: usize) -> Vec<Vec<f32>> {
    stft(data, n_fft, hop).frames
}

#[cfg(test)]
mod tests {
    use super::{stft, MAX_FRAMES};

    const SR: usize = 44_100;

    #[test]
    fn short_file_keeps_every_frame_at_the_analysis_hop() {
        let data = vec![0.1f32; SR * 10];
        let s = stft(&data, 2048, 512);
        assert_eq!(s.hop, 512);
        assert_eq!(s.frames.len(), 1 + (data.len() - 2048) / 512);
        assert!(s.next.is_empty());
    }

    #[test]
    fn long_file_frames_span_the_whole_duration() {
        // ROAD-3445: a 120 s file used to stop at frame 3000 (~35 s).
        let data = vec![0.1f32; SR * 120];
        let s = stft(&data, 2048, 512);
        assert!(s.frames.len() <= MAX_FRAMES);
        assert_eq!(s.next.len(), s.frames.len());
        let last_start_s = (s.frames.len() - 1) as f64 * s.hop as f64 / SR as f64;
        assert!(last_start_s > 100.0, "last frame starts at {last_start_s:.1} s");
        assert!(last_start_s < 120.0);
    }
}
