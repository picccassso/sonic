use opus_pure::{
    Application, MAX_PACKET_BYTES, OggOpusWriter, OpusEncoder, OpusHead, OpusTags,
};

use crate::{
    audio::{metadata::AudioMetadata, pcm::PcmAudio, resample},
    errors::TranscodeError,
};

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

    // Opus operates at 48 kHz. Resample if necessary.
    let pcm_48k = if pcm.sample_rate != 48_000 {
        resample::resample_to_48k(pcm)?
    } else {
        pcm.clone()
    };

    let channels = pcm_48k.channels as usize;
    let total_48k_frames = pcm_48k.frames();

    let mut encoder = OpusEncoder::new(48_000, channels, Application::Audio)
        .map_err(|err| TranscodeError::Encode(format!("opus encoder init failed: {err}")))?;

    encoder.bitrate_bps = (bitrate_kbps.saturating_mul(1000)) as i32;

    let head = OpusHead::for_encoder(&encoder, pcm.sample_rate);
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

    let mut writer = OggOpusWriter::with_tags(Vec::new(), head.clone(), tags)
        .map_err(|err| TranscodeError::Encode(format!("ogg opus writer init failed: {err}")))?;

    let mut packet = vec![0u8; MAX_PACKET_BYTES];
    // 20ms frame at 48kHz = 960 samples per channel
    let frame_samples = 960usize;
    let chunk_size = frame_samples * channels;

    // RFC 7845: Flush the encoder's pre-skip delay with trailing silence,
    // and set the final granule position to pre_skip + original audio frames.
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
                .map_err(|err| {
                    TranscodeError::Encode(format!("ogg packet write failed: {err}"))
                })?;
        } else {
            writer
                .write_packet(&packet[..bytes])
                .map_err(|err| {
                    TranscodeError::Encode(format!("ogg packet write failed: {err}"))
                })?;
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
    use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, Trim};
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
            let n = decoder.decode(&packet.data, MAX_PACKET_SAMPLES, &mut block).unwrap();
            let kept = trim.keep(&packet, &block[..n * 2]);
            decoded_frames += kept.len() / 2;
        }

        let expected_48k_frames = (48_000.0 * duration_secs) as usize;
        assert!((decoded_frames as i64 - expected_48k_frames as i64).abs() <= 50);
    }
}


