use rubato::{
    audioadapter_buffers::{direct::InterleavedSlice, owned::InterleavedOwned},
    Fft, FixedSync, Indexing, Resampler,
};

use crate::{audio::pcm::PcmAudio, errors::TranscodeError};

/// Chunked FFT resampler that converts interleaved i16 PCM to a target sample rate
/// in constant memory instead of buffering an entire track.
pub struct StreamingResampler {
    resampler: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out_f32: Vec<f32>,
    out_frames: usize,
    out_i16: Vec<i16>,
    total_input_frames: usize,
    total_output_frames: usize,
    frames_to_trim: usize,
}

impl StreamingResampler {
    pub fn new(
        input_sample_rate: u32,
        target_sample_rate: u32,
        channels: u16,
    ) -> Result<Self, TranscodeError> {
        if channels == 0 {
            return Err(TranscodeError::Encode(
                "cannot resample audio with 0 channels".to_string(),
            ));
        }
        if input_sample_rate == 0 || target_sample_rate == 0 {
            return Err(TranscodeError::Encode(
                "sample rate must be greater than 0".to_string(),
            ));
        }

        let ch = channels as usize;
        let resampler = Fft::<f32>::new(
            input_sample_rate as usize,
            target_sample_rate as usize,
            1024,
            ch,
            FixedSync::Both,
        )
        .map_err(|err| TranscodeError::Encode(format!("resampler init failed: {err}")))?;

        let out_frames = resampler.output_frames_max();
        let frames_to_trim = resampler.output_delay();

        Ok(Self {
            resampler,
            channels: ch,
            pending: Vec::with_capacity(1024 * ch * 2),
            out_f32: vec![0.0; out_frames * ch],
            out_frames,
            out_i16: Vec::with_capacity(out_frames * ch),
            total_input_frames: 0,
            total_output_frames: 0,
            frames_to_trim,
        })
    }

    pub fn push(
        &mut self,
        samples: &[i16],
        mut sink: impl FnMut(&[i16]) -> Result<(), TranscodeError>,
    ) -> Result<(), TranscodeError> {
        let usable = samples.len() - (samples.len() % self.channels);
        if usable == 0 {
            return Ok(());
        }
        self.pending
            .extend(samples[..usable].iter().map(|&s| s as f32 / 32768.0));
        self.total_input_frames += usable / self.channels;

        let mut needed_frames = self.resampler.input_frames_next();
        while self.pending.len() / self.channels >= needed_frames {
            let needed_samples = needed_frames * self.channels;
            let (nbr_in, nbr_out) = {
                let input_adapter = InterleavedSlice::new(
                    &self.pending[..needed_samples],
                    self.channels,
                    needed_frames,
                )
                .map_err(|err| TranscodeError::Encode(format!("resampler buffer error: {err}")))?;
                let mut output_adapter =
                    InterleavedSlice::new_mut(&mut self.out_f32, self.channels, self.out_frames)
                        .map_err(|err| {
                            TranscodeError::Encode(format!("resampler buffer error: {err}"))
                        })?;
                self.resampler
                    .process_into_buffer(&input_adapter, &mut output_adapter, None)
                    .map_err(|err| TranscodeError::Encode(format!("resampling failed: {err}")))?
            };

            self.pending.drain(..nbr_in * self.channels);
            self.emit_frames(nbr_out, None, &mut sink)?;
            needed_frames = self.resampler.input_frames_next();
        }

        Ok(())
    }

    pub fn finish(
        mut self,
        mut sink: impl FnMut(&[i16]) -> Result<(), TranscodeError>,
    ) -> Result<(), TranscodeError> {
        if self.total_input_frames == 0 {
            return Ok(());
        }

        let expected_output =
            (self.resampler.resample_ratio() * self.total_input_frames as f64).ceil() as usize;

        let rem_frames = self.pending.len() / self.channels;
        if rem_frames > 0 {
            let needed_frames = self.resampler.input_frames_next();
            self.pending.resize(needed_frames * self.channels, 0.0);
            let (_, nbr_out) = {
                let input_adapter =
                    InterleavedSlice::new(&self.pending, self.channels, needed_frames).map_err(
                        |err| TranscodeError::Encode(format!("resampler buffer error: {err}")),
                    )?;
                let mut output_adapter =
                    InterleavedSlice::new_mut(&mut self.out_f32, self.channels, self.out_frames)
                        .map_err(|err| {
                            TranscodeError::Encode(format!("resampler buffer error: {err}"))
                        })?;
                let indexing = Indexing {
                    input_offset: 0,
                    output_offset: 0,
                    active_channels_mask: None,
                    partial_len: Some(rem_frames),
                };
                self.resampler
                    .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
                    .map_err(|err| TranscodeError::Encode(format!("resampling failed: {err}")))?
            };
            self.emit_frames(nbr_out, Some(expected_output), &mut sink)?;
        }

        let needed_frames = self.resampler.input_frames_next();
        let zeros = vec![0.0_f32; needed_frames * self.channels];
        let zeros_adapter = InterleavedSlice::new(&zeros, self.channels, needed_frames)
            .map_err(|err| TranscodeError::Encode(format!("resampler buffer error: {err}")))?;
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            active_channels_mask: None,
            partial_len: Some(0),
        };

        while self.total_output_frames < expected_output {
            let (_, nbr_out) = {
                let mut output_adapter =
                    InterleavedSlice::new_mut(&mut self.out_f32, self.channels, self.out_frames)
                        .map_err(|err| {
                            TranscodeError::Encode(format!("resampler buffer error: {err}"))
                        })?;
                self.resampler
                    .process_into_buffer(&zeros_adapter, &mut output_adapter, Some(&indexing))
                    .map_err(|err| TranscodeError::Encode(format!("resampling failed: {err}")))?
            };
            if nbr_out == 0 {
                break;
            }
            self.emit_frames(nbr_out, Some(expected_output), &mut sink)?;
        }

        Ok(())
    }

    fn emit_frames(
        &mut self,
        nbr_out: usize,
        max_total_frames: Option<usize>,
        sink: &mut impl FnMut(&[i16]) -> Result<(), TranscodeError>,
    ) -> Result<(), TranscodeError> {
        let skip = self.frames_to_trim.min(nbr_out);
        self.frames_to_trim -= skip;
        let mut avail = nbr_out - skip;
        if let Some(limit) = max_total_frames {
            avail = avail.min(limit.saturating_sub(self.total_output_frames));
        }
        if avail == 0 {
            return Ok(());
        }

        let start = skip * self.channels;
        let end = (skip + avail) * self.channels;
        self.out_i16.clear();
        self.out_i16.extend(
            self.out_f32[start..end]
                .iter()
                .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16),
        );
        self.total_output_frames += avail;
        sink(&self.out_i16)
    }
}

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

    Ok(PcmAudio::new(output_i16, target_sample_rate, pcm.channels))
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
