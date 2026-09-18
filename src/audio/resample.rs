use rubato::{
    audioadapter_buffers::owned::InterleavedOwned, Fft, FixedSync, Resampler,
};

use crate::{audio::pcm::PcmAudio, errors::TranscodeError};

/// Resamples PCM audio to a target sample rate using high-fidelity FFT sinc interpolation.
/// If `pcm.sample_rate == target_sample_rate`, this returns a clone of the original audio.
pub fn resample(pcm: &PcmAudio, target_sample_rate: u32) -> Result<PcmAudio, TranscodeError> {
    if pcm.sample_rate == target_sample_rate {
        return Ok(pcm.clone());
    }

    if pcm.channels == 0 {
        return Err(TranscodeError::Encode(
            "cannot resample audio with 0 channels".to_string(),
        ));
    }

    if pcm.sample_rate == 0 || target_sample_rate == 0 {
        return Err(TranscodeError::Encode(
            "sample rate must be greater than 0".to_string(),
        ));
    }

    if pcm.samples.is_empty() {
        return Ok(PcmAudio::new(Vec::new(), target_sample_rate, pcm.channels));
    }

    let ch = pcm.channels as usize;
    let input_frames = pcm.samples.len() / ch;
    if input_frames == 0 {
        return Ok(PcmAudio::new(Vec::new(), target_sample_rate, pcm.channels));
    }

    // Convert i16 samples to normalized f32 (-1.0 to 1.0)
    let mut input_f32 = Vec::with_capacity(pcm.samples.len());
    for &sample in &pcm.samples {
        input_f32.push(sample as f32 / 32768.0);
    }

    let input_adapter = InterleavedOwned::new_from(input_f32, ch, input_frames)
        .map_err(|err| TranscodeError::Encode(format!("resampler buffer error: {err}")))?;

    // Use FFT resampler for maximum fidelity and throughput
    let chunk_size = 1024usize;
    let mut resampler = Fft::<f32>::new(
        pcm.sample_rate as usize,
        target_sample_rate as usize,
        chunk_size,
        ch,
        FixedSync::Both,
    )
    .map_err(|err| TranscodeError::Encode(format!("resampler init failed: {err}")))?;

    let output_adapter = resampler
        .process_all(&input_adapter, input_frames, None)
        .map_err(|err| TranscodeError::Encode(format!("resampling failed: {err}")))?;

    let output_f32 = output_adapter.take_data();
    let mut output_i16 = Vec::with_capacity(output_f32.len());

    for &sample in &output_f32 {
        let clamped = (sample * 32767.0).clamp(-32768.0, 32767.0);
        output_i16.push(clamped as i16);
    }

    Ok(PcmAudio::new(
        output_i16,
        target_sample_rate,
        pcm.channels,
    ))
}

/// Resamples PCM audio to 48,000 Hz, which is the required native rate for Opus.
pub fn resample_to_48k(pcm: &PcmAudio) -> Result<PcmAudio, TranscodeError> {
    resample(pcm, 48_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_when_rate_matches() {
        let original = PcmAudio::new(vec![100, -200, 300, -400], 48_000, 2);
        let resampled = resample_to_48k(&original).unwrap();
        assert_eq!(resampled, original);
    }

    #[test]
    fn resamples_44100_to_48000() {
        let sample_rate = 44_100;
        let channels = 2;
        let duration_secs = 0.1;
        let num_frames = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = Vec::with_capacity(num_frames * channels);

        for i in 0..num_frames {
            let t = i as f32 / sample_rate as f32;
            let val = ((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 10000.0) as i16;
            samples.push(val);
            samples.push(val);
        }

        let pcm = PcmAudio::new(samples, sample_rate, channels as u16);
        let resampled = resample_to_48k(&pcm).unwrap();

        assert_eq!(resampled.sample_rate, 48_000);
        assert_eq!(resampled.channels, 2);
        let expected_frames = (48_000.0 * duration_secs) as usize;
        let actual_frames = resampled.frames();
        assert!((actual_frames as i64 - expected_frames as i64).abs() <= 5);
    }

    #[test]
    fn resamples_96000_to_48000() {
        let sample_rate = 96_000;
        let channels = 1;
        let duration_secs = 0.1;
        let num_frames = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let t = i as f32 / sample_rate as f32;
            let val = ((t * 1000.0 * 2.0 * std::f32::consts::PI).sin() * 15000.0) as i16;
            samples.push(val);
        }

        let pcm = PcmAudio::new(samples, sample_rate, channels as u16);
        let resampled = resample_to_48k(&pcm).unwrap();

        assert_eq!(resampled.sample_rate, 48_000);
        assert_eq!(resampled.channels, 1);
        let expected_frames = (48_000.0 * duration_secs) as usize;
        let actual_frames = resampled.frames();
        assert!((actual_frames as i64 - expected_frames as i64).abs() <= 5);
    }
}
