use std::io::Cursor;

use claxon::FlacReader;
use hound::{SampleFormat as WavSampleFormat, WavReader};
use minimp3::{Decoder, Error as Mp3Error};

use crate::{
    audio::{
        detect::{self, InputFormat},
        metadata,
    },
    errors::TranscodeError,
};

#[derive(Debug, Clone)]
pub struct AudioInfo {
    pub input_format: InputFormat,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub duration_ms: u64,
    pub total_samples_per_channel: u64,
    pub has_metadata: bool,
    pub has_artwork: bool,
}

pub fn probe(input: &[u8]) -> Result<AudioInfo, TranscodeError> {
    let input_format = detect::detect_format(input)?;
    match input_format {
        InputFormat::Mp3 => probe_mp3(input),
        InputFormat::Wav => probe_wav(input),
        InputFormat::Flac => probe_flac(input),
    }
}

fn probe_mp3(input: &[u8]) -> Result<AudioInfo, TranscodeError> {
    let mut decoder = Decoder::new(Cursor::new(input));
    let metadata = metadata::extract_mp3_metadata(input);
    let mut sample_rate = None;
    let mut channels = None;
    let mut total_samples = 0_u64;

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                let frame_rate = frame.sample_rate as u32;
                let frame_channels = frame.channels as u16;

                sample_rate.get_or_insert(frame_rate);
                channels.get_or_insert(frame_channels);

                if sample_rate != Some(frame_rate) || channels != Some(frame_channels) {
                    return Err(TranscodeError::Decode(
                        "mp3 stream changed sample rate or channels mid-stream".to_string(),
                    ));
                }

                if frame_channels > 0 {
                    total_samples += (frame.data.len() / frame_channels as usize) as u64;
                }
            }
            Err(Mp3Error::Eof) => break,
            Err(err) => return Err(TranscodeError::Decode(format!("mp3 probe failed: {err}"))),
        }
    }

    let sample_rate = sample_rate.ok_or_else(|| {
        TranscodeError::Decode("mp3 stream did not contain decodable frames".to_string())
    })?;
    let channels = channels.unwrap_or_default();
    let duration_ms = samples_to_ms(total_samples, sample_rate);

    Ok(AudioInfo {
        input_format: InputFormat::Mp3,
        sample_rate,
        channels,
        bits_per_sample: 16,
        duration_ms,
        total_samples_per_channel: total_samples,
        has_metadata: metadata.as_ref().map(|m| !m.is_empty()).unwrap_or(false),
        has_artwork: metadata.and_then(|m| m.artwork).is_some(),
    })
}

fn probe_wav(input: &[u8]) -> Result<AudioInfo, TranscodeError> {
    let reader = WavReader::new(Cursor::new(input))
        .map_err(|err| TranscodeError::Decode(format!("wav probe failed: {err}")))?;
    let spec = reader.spec();
    let total_samples = u64::from(reader.duration());

    Ok(AudioInfo {
        input_format: InputFormat::Wav,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: match spec.sample_format {
            WavSampleFormat::Int => spec.bits_per_sample,
            WavSampleFormat::Float => 32,
        },
        duration_ms: samples_to_ms(total_samples, spec.sample_rate),
        total_samples_per_channel: total_samples,
        has_metadata: false,
        has_artwork: false,
    })
}

fn probe_flac(input: &[u8]) -> Result<AudioInfo, TranscodeError> {
    let reader = FlacReader::new(Cursor::new(input))
        .map_err(|err| TranscodeError::Decode(format!("flac probe failed: {err}")))?;
    let info = reader.streaminfo();
    let total_samples = info.samples.unwrap_or(0);

    Ok(AudioInfo {
        input_format: InputFormat::Flac,
        sample_rate: info.sample_rate,
        channels: info.channels as u16,
        bits_per_sample: info.bits_per_sample as u16,
        duration_ms: samples_to_ms(total_samples, info.sample_rate),
        total_samples_per_channel: total_samples,
        has_metadata: false,
        has_artwork: false,
    })
}

fn samples_to_ms(samples_per_channel: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    samples_per_channel.saturating_mul(1000) / u64::from(sample_rate)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use hound::{SampleFormat, WavSpec, WavWriter};

    use super::probe;
    use crate::audio::detect::InputFormat;

    #[test]
    fn probes_wav_without_decoding_to_output() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let spec = WavSpec {
                channels: 2,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::new(&mut cursor, spec).expect("create wav writer");
            for _ in 0..44_100 {
                writer.write_sample::<i16>(0).expect("write left");
                writer.write_sample::<i16>(0).expect("write right");
            }
            writer.finalize().expect("finalize wav");
        }

        let bytes = cursor.into_inner();
        let info = probe(&bytes).expect("probe wav");

        assert_eq!(info.input_format, InputFormat::Wav);
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.duration_ms, 1000);
        assert_eq!(info.total_samples_per_channel, 44_100);
        assert!(!info.has_metadata);
        assert!(!info.has_artwork);
    }
}
