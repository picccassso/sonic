use std::io::Write;

use opus_pure::{Application, OggOpusWriter, OpusEncoder, OpusHead, OpusTags, MAX_PACKET_BYTES};

use crate::{
    audio::{
        metadata::AudioMetadata,
        pcm::PcmAudio,
        resample::{self, StreamingResampler},
    },
    errors::TranscodeError,
};

const OPUS_FRAME_SAMPLES: usize = 960;

fn build_opus_tags(metadata: Option<&AudioMetadata>) -> OpusTags {
    let mut tags = OpusTags::new();
    if let Some(meta) = metadata {
        if let Some(ref title) = meta.title {
            let _ = tags.push("TITLE", title);
        }
        if let Some(ref artist) = meta.artist {
            let _ = tags.push("ARTIST", artist);
        }
        if let Some(ref album) = meta.album {
            let _ = tags.push("ALBUM", album);
        }
        if let Some(ref genre) = meta.genre {
            let _ = tags.push("GENRE", genre);
        }
        if let Some(year) = meta.year {
            let _ = tags.push("DATE", &year.to_string());
        }
        if let Some(track) = meta.track {
            let _ = tags.push("TRACKNUMBER", &track.to_string());
        }
    }
    tags
}

/// Streaming Ogg Opus encoder that resamples (when needed) and encodes chunks
/// in constant memory directly into a `Write` sink.
pub struct OpusStreamEncoder<W: Write> {
    encoder: OpusEncoder,
    writer: OggOpusWriter<W>,
    resampler: Option<StreamingResampler>,
    channels: usize,
    pre_skip: u16,
    total_48k_frames: u64,
    pending_48k: Vec<i16>,
    packet: Vec<u8>,
}

impl<W: Write> OpusStreamEncoder<W> {
    pub fn new(
        output: W,
        input_sample_rate: u32,
        channels: u16,
        bitrate_kbps: u32,
        metadata: Option<&AudioMetadata>,
    ) -> Result<Self, TranscodeError> {
        if channels == 0 || channels > 2 {
            return Err(TranscodeError::Encode(format!(
                "unsupported channel count for Opus encoding: {channels} (expected mono or stereo)"
            )));
        }
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode(
                "bitrate must be greater than 0".to_string(),
            ));
        }

        let ch = channels as usize;
        let resampler = if input_sample_rate != 48_000 {
            Some(StreamingResampler::new(
                input_sample_rate,
                48_000,
                channels,
            )?)
        } else {
            None
        };

        let mut encoder = OpusEncoder::new(48_000, ch, Application::Audio)
            .map_err(|err| TranscodeError::Encode(format!("opus encoder init failed: {err}")))?;
        encoder.bitrate_bps = (bitrate_kbps.saturating_mul(1000)) as i32;

        let head = OpusHead::for_encoder(&encoder, input_sample_rate);
        let pre_skip = head.pre_skip;
        let writer = OggOpusWriter::with_tags(output, head, build_opus_tags(metadata))
            .map_err(|err| TranscodeError::Encode(format!("ogg opus writer init failed: {err}")))?;

        Ok(Self {
            encoder,
            writer,
            resampler,
            channels: ch,
            pre_skip,
            total_48k_frames: 0,
            pending_48k: Vec::with_capacity(OPUS_FRAME_SAMPLES * ch * 2),
            packet: vec![0u8; MAX_PACKET_BYTES],
        })
    }

    pub fn push(&mut self, samples: &[i16]) -> Result<(), TranscodeError> {
        if let Some(mut resampler) = self.resampler.take() {
            let res = resampler.push(samples, |resampled| self.push_48k(resampled));
            self.resampler = Some(resampler);
            res
        } else {
            self.push_48k(samples)
        }
    }

    fn push_48k(&mut self, samples_48k: &[i16]) -> Result<(), TranscodeError> {
        let usable = samples_48k.len() - (samples_48k.len() % self.channels);
        if usable == 0 {
            return Ok(());
        }
        self.pending_48k.extend_from_slice(&samples_48k[..usable]);
        self.total_48k_frames += (usable / self.channels) as u64;

        let chunk_size = OPUS_FRAME_SAMPLES * self.channels;
        let mut consumed = 0usize;
        while self.pending_48k.len() - consumed >= chunk_size {
            let bytes = self
                .encoder
                .encode_s16(
                    &self.pending_48k[consumed..consumed + chunk_size],
                    OPUS_FRAME_SAMPLES,
                    &mut self.packet,
                )
                .map_err(|err| TranscodeError::Encode(format!("opus encode failed: {err}")))?;
            self.writer
                .write_packet(&self.packet[..bytes])
                .map_err(|err| TranscodeError::Encode(format!("ogg packet write failed: {err}")))?;
            consumed += chunk_size;
        }
        if consumed > 0 {
            self.pending_48k.drain(..consumed);
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, TranscodeError> {
        if let Some(resampler) = self.resampler.take() {
            resampler.finish(|resampled| self.push_48k(resampled))?;
        }

        let chunk_size = OPUS_FRAME_SAMPLES * self.channels;
        let final_granule = u64::from(self.pre_skip) + self.total_48k_frames;

        // Append lookahead silence, then pad up to a full 20ms packet boundary.
        let lookahead_samples = usize::from(self.pre_skip) * self.channels;
        self.pending_48k
            .resize(self.pending_48k.len() + lookahead_samples, 0);
        let rem = self.pending_48k.len() % chunk_size;
        if rem != 0 || self.pending_48k.is_empty() {
            self.pending_48k
                .resize(self.pending_48k.len() + (chunk_size - rem), 0);
        }

        let num_packets = self.pending_48k.len() / chunk_size;
        for i in 0..num_packets {
            let start = i * chunk_size;
            let bytes = self
                .encoder
                .encode_s16(
                    &self.pending_48k[start..start + chunk_size],
                    OPUS_FRAME_SAMPLES,
                    &mut self.packet,
                )
                .map_err(|err| TranscodeError::Encode(format!("opus encode failed: {err}")))?;

            if i + 1 == num_packets {
                let duration = final_granule.saturating_sub(self.writer.granule() as u64);
                self.writer
                    .write_packet_with_duration(&self.packet[..bytes], duration as u32)
                    .map_err(|err| {
                        TranscodeError::Encode(format!("ogg packet write failed: {err}"))
                    })?;
            } else {
                self.writer.write_packet(&self.packet[..bytes]).map_err(|err| {
                    TranscodeError::Encode(format!("ogg packet write failed: {err}"))
                })?;
            }
        }

        self.writer
            .finish()
            .map_err(|err| TranscodeError::Encode(format!("ogg opus finish failed: {err}")))
    }
}

/// Encodes PCM audio into an RFC 7845 compliant Ogg Opus (.opus) byte stream.
///
/// Automatically resamples non-48 kHz audio to 48 kHz prior to encoding, preserves
/// Vorbis comments from `AudioMetadata`, and properly sets the pre-skip and end-trim
/// in the Ogg container for gapless playback.
pub fn encode_opus(
    pcm: &PcmAudio,
    bitrate_kbps: u32,
    metadata: Option<&AudioMetadata>,
) -> Result<Vec<u8>, TranscodeError> {
    if pcm.channels == 0 || pcm.channels > 2 {
        return Err(TranscodeError::Encode(format!(
            "unsupported channel count for Opus encoding: {} (expected mono or stereo)",
            pcm.channels
        )));
    }

    if bitrate_kbps == 0 {
        return Err(TranscodeError::Encode(
            "bitrate must be greater than 0".to_string(),
        ));
    }

    // Opus operates at 48 kHz. Borrow directly when already 48 kHz to avoid cloning PCM.
    let resampled;
    let pcm_48k = if pcm.sample_rate != 48_000 {
        resampled = resample::resample_to_48k(pcm)?;
        &resampled
    } else {
        pcm
    };

    let channels = pcm_48k.channels as usize;
    let total_48k_frames = pcm_48k.frames();

    let mut encoder = OpusEncoder::new(48_000, channels, Application::Audio)
        .map_err(|err| TranscodeError::Encode(format!("opus encoder init failed: {err}")))?;

    encoder.bitrate_bps = (bitrate_kbps.saturating_mul(1000)) as i32;

    let head = OpusHead::for_encoder(&encoder, pcm.sample_rate);
    let tags = build_opus_tags(metadata);

    let mut writer = OggOpusWriter::with_tags(Vec::new(), head.clone(), tags)
        .map_err(|err| TranscodeError::Encode(format!("ogg opus writer init failed: {err}")))?;

    let mut packet = vec![0u8; MAX_PACKET_BYTES];
    let frame_samples = OPUS_FRAME_SAMPLES;
    let chunk_size = frame_samples * channels;

    let lookahead = head.pre_skip as usize;
    let total_frames_to_encode = total_48k_frames + lookahead;
    let num_packets = total_frames_to_encode.div_ceil(frame_samples);
    let final_granule = u64::from(head.pre_skip) + total_48k_frames as u64;

    let mut block = vec![0i16; chunk_size];

    for i in 0..num_packets {
        let start = (i * chunk_size).min(pcm_48k.samples.len());
        let end = (start + chunk_size).min(pcm_48k.samples.len());
        let copied = end - start;
        if copied > 0 {
            block[..copied].copy_from_slice(&pcm_48k.samples[start..end]);
        }
        if copied < chunk_size {
            block[copied..].fill(0);
        }

        let bytes = encoder
            .encode_s16(&block, frame_samples, &mut packet)
            .map_err(|err| TranscodeError::Encode(format!("opus encode failed: {err}")))?;

        if i + 1 == num_packets {
            let duration = final_granule.saturating_sub(writer.granule() as u64);
            writer
                .write_packet_with_duration(&packet[..bytes], duration as u32)
                .map_err(|err| TranscodeError::Encode(format!("ogg packet write failed: {err}")))?;
        } else {
            writer
                .write_packet(&packet[..bytes])
                .map_err(|err| TranscodeError::Encode(format!("ogg packet write failed: {err}")))?;
        }
    }

    let output = writer
        .finish()
        .map_err(|err| TranscodeError::Encode(format!("ogg opus finish failed: {err}")))?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opus_pure::{OggOpusReader, Trim, MAX_PACKET_SAMPLES};
    use std::io::Cursor;

    #[test]
    fn encodes_and_decodes_stereo_opus_with_metadata() {
        let sample_rate = 44_100;
        let channels = 2;
        let duration_secs = 0.2;
        let num_frames = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = Vec::with_capacity(num_frames * channels);

        for i in 0..num_frames {
            let t = i as f32 / sample_rate as f32;
            let val = ((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 12000.0) as i16;
            samples.push(val);
            samples.push(val);
        }

        let pcm = PcmAudio::new(samples, sample_rate, channels as u16);
        let metadata = AudioMetadata {
            title: Some("Ariami Opus Test".to_string()),
            artist: Some("Sonic Transcoder".to_string()),
            album: Some("Sonic Album".to_string()),
            genre: Some("Electronic".to_string()),
            year: Some(2026),
            track: Some(1),
            artwork: None,
        };

        let opus_bytes = encode_opus(&pcm, 192, Some(&metadata)).unwrap();
        assert!(!opus_bytes.is_empty());
        assert_eq!(&opus_bytes[..4], b"OggS");

        // Validate Ogg Opus container structure and Vorbis comments
        let mut reader = OggOpusReader::new(Cursor::new(&opus_bytes)).unwrap();
        let head = reader.head().clone();
        assert_eq!(head.channel_count, 2);
        assert_eq!(head.input_sample_rate, 44_100);

        let tags = reader.tags();
        assert_eq!(tags.get("TITLE"), Some("Ariami Opus Test"));
        assert_eq!(tags.get("ARTIST"), Some("Sonic Transcoder"));
        assert_eq!(tags.get("ALBUM"), Some("Sonic Album"));
        assert_eq!(tags.get("GENRE"), Some("Electronic"));
        assert_eq!(tags.get("DATE"), Some("2026"));
        assert_eq!(tags.get("TRACKNUMBER"), Some("1"));

        // Validate decoding back to PCM
        let mut decoder = head.decoder(48_000).unwrap();
        let mut trim = Trim::new(&head, 48_000, 2).unwrap();
        let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * 2];
        let mut decoded_frames = 0;

        for packet in reader.packets() {
            let packet = packet.unwrap();
            let n = decoder
                .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
                .unwrap();
            let kept = trim.keep(&packet, &block[..n * 2]);
            decoded_frames += kept.len() / 2;
        }

        let expected_48k_frames = (48_000.0 * duration_secs) as usize;
        assert!((decoded_frames as i64 - expected_48k_frames as i64).abs() <= 50);
    }
}
