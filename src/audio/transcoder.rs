use std::io::Cursor;

use claxon::FlacReader;
use hound::{SampleFormat as WavSampleFormat, WavReader};
use minimp3::{Decoder, Error as Mp3Error};
use mp3lame_encoder::{
    max_required_buffer_size, Bitrate as Mp3Bitrate, Builder as Mp3Builder, FlushNoGap,
    InterleavedPcm, Mode as Mp3Mode, MonoPcm, Quality as Mp3Quality,
};

use crate::{
    audio::{
        detect::{self, InputFormat},
        m4a,
        metadata::{self, AudioMetadata},
        output::OutputFormat,
        preset::QualityPreset,
    },
    errors::TranscodeError,
};
#[cfg(feature = "aac-fdk")]
use fdk_aac::enc::{
    AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport,
};

#[derive(Debug, Clone)]
pub struct Transcoder {
    bitrate_kbps: u32,
}

#[derive(Debug, Clone)]
struct PcmAudio {
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
}

impl Transcoder {
    pub fn new(bitrate_kbps: u32) -> Self {
        Self { bitrate_kbps }
    }

    pub fn default_bitrate_kbps(&self) -> u32 {
        self.bitrate_kbps
    }

    pub fn transcode_with_preset(
        &self,
        input: &[u8],
        preset: QualityPreset,
    ) -> Result<Vec<u8>, TranscodeError> {
        self.transcode_with_preset_and_format(input, preset, OutputFormat::Aac)
    }

    pub fn transcode_with_preset_and_format(
        &self,
        input: &[u8],
        preset: QualityPreset,
        output_format: OutputFormat,
    ) -> Result<Vec<u8>, TranscodeError> {
        self.transcode_with_bitrate_and_format(input, preset.bitrate_kbps(), output_format)
    }

    pub fn transcode(&self, input: &[u8]) -> Result<Vec<u8>, TranscodeError> {
        self.transcode_with_bitrate_and_format(input, self.bitrate_kbps, OutputFormat::Aac)
    }

    pub fn transcode_with_bitrate(
        &self,
        input: &[u8],
        bitrate_kbps: u32,
    ) -> Result<Vec<u8>, TranscodeError> {
        self.transcode_with_bitrate_and_format(input, bitrate_kbps, OutputFormat::Aac)
    }

    pub fn transcode_with_bitrate_and_format(
        &self,
        input: &[u8],
        bitrate_kbps: u32,
        output_format: OutputFormat,
    ) -> Result<Vec<u8>, TranscodeError> {
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode("bitrate must be > 0".to_string()));
        }

        let input_format = detect::detect_format(input)?;
        let metadata = if input_format == InputFormat::Mp3 {
            metadata::extract_mp3_metadata(input)
        } else {
            None
        };

        let pcm = match input_format {
            InputFormat::Mp3 => self.decode_mp3(input)?,
            InputFormat::Wav => self.decode_wav(input)?,
            InputFormat::Flac => self.decode_flac(input)?,
        };

        let mut output = match output_format {
            OutputFormat::Aac => self.encode_aac(&pcm, bitrate_kbps)?,
            OutputFormat::M4a => {
                let adts = self.encode_aac(&pcm, bitrate_kbps)?;
                m4a::adts_to_m4a(&adts, bitrate_kbps)?
            }
            OutputFormat::Mp3 => self.encode_mp3(&pcm, bitrate_kbps)?,
        };

        if let Some(metadata) = metadata {
            output = apply_metadata(output, &metadata, output_format)?;
        }

        Ok(output)
    }

    fn decode_mp3(&self, input: &[u8]) -> Result<PcmAudio, TranscodeError> {
        let mut decoder = Decoder::new(Cursor::new(input));

        let mut samples: Vec<i16> = Vec::new();
        let mut sample_rate: Option<u32> = None;
        let mut channels: Option<u16> = None;

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

                    samples.extend_from_slice(&frame.data);
                }
                Err(Mp3Error::Eof) => break,
                Err(err) => return Err(TranscodeError::Decode(format!("mp3 decode failed: {err}"))),
            }
        }

        if samples.is_empty() {
            return Err(TranscodeError::Decode(
                "mp3 stream did not contain decodable frames".to_string(),
            ));
        }

        Ok(PcmAudio {
            samples,
            sample_rate: sample_rate.unwrap_or_default(),
            channels: channels.unwrap_or_default(),
        })
    }

    fn decode_wav(&self, input: &[u8]) -> Result<PcmAudio, TranscodeError> {
        let mut reader = WavReader::new(Cursor::new(input))
            .map_err(|err| TranscodeError::Decode(format!("wav decode failed: {err}")))?;
        let spec = reader.spec();

        if spec.channels == 0 || spec.channels > 2 {
            return Err(TranscodeError::Decode(format!(
                "unsupported wav channel count: {} (expected mono/stereo)",
                spec.channels
            )));
        }

        let bits = spec.bits_per_sample as u32;
        let samples: Result<Vec<i16>, TranscodeError> = match spec.sample_format {
            WavSampleFormat::Int => {
                if bits <= 16 {
                    reader
                        .samples::<i16>()
                        .map(|s| {
                            s.map(|sample| scale_i32_to_i16(sample as i32, bits)).map_err(|err| {
                                TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                            })
                        })
                        .collect()
                } else {
                    reader
                        .samples::<i32>()
                        .map(|s| {
                            s.map(|sample| scale_i32_to_i16(sample, bits)).map_err(|err| {
                                TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                            })
                        })
                        .collect()
                }
            }
            WavSampleFormat::Float => reader
                .samples::<f32>()
                .map(|s| {
                    s.map(f32_to_i16)
                        .map_err(|err| TranscodeError::Decode(format!("wav sample decode failed: {err}")))
                })
                .collect(),
        };

        let samples = samples?;
        if samples.is_empty() {
            return Err(TranscodeError::Decode(
                "wav stream did not contain decodable samples".to_string(),
            ));
        }

        Ok(PcmAudio {
            samples,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
        })
    }

    fn decode_flac(&self, input: &[u8]) -> Result<PcmAudio, TranscodeError> {
        let mut reader = FlacReader::new(Cursor::new(input))
            .map_err(|err| TranscodeError::Decode(format!("flac decode failed: {err}")))?;
        let info = reader.streaminfo();

        if info.channels == 0 || info.channels > 2 {
            return Err(TranscodeError::Decode(format!(
                "unsupported flac channel count: {} (expected mono/stereo)",
                info.channels
            )));
        }

        let bits = u32::from(info.bits_per_sample);
        let mut samples = Vec::new();

        for sample in reader.samples() {
            let sample = sample
                .map_err(|err| TranscodeError::Decode(format!("flac sample decode failed: {err}")))?;
            samples.push(scale_i32_to_i16(sample, bits));
        }

        if samples.is_empty() {
            return Err(TranscodeError::Decode(
                "flac stream did not contain decodable samples".to_string(),
            ));
        }

        Ok(PcmAudio {
            samples,
            sample_rate: info.sample_rate,
            channels: info.channels as u16,
        })
    }

    fn encode_aac(&self, pcm: &PcmAudio, bitrate_kbps: u32) -> Result<Vec<u8>, TranscodeError> {
        #[cfg(feature = "aac-fdk")]
        {
            let channels = match pcm.channels {
                1 => ChannelMode::Mono,
                2 => ChannelMode::Stereo,
                other => {
                    return Err(TranscodeError::Encode(format!(
                        "unsupported channel count for AAC encoding: {other}"
                    )))
                }
            };

            let encoder = Encoder::new(EncoderParams {
                bit_rate: BitRate::Cbr(bitrate_kbps.saturating_mul(1000)),
                sample_rate: pcm.sample_rate,
                transport: Transport::Adts,
                channels,
                audio_object_type: AudioObjectType::Mpeg4LowComplexity,
            })
            .map_err(|err| TranscodeError::Encode(format!("aac encoder init failed: {err}")))?;

            let mut output = Vec::with_capacity(pcm.samples.len() / 2);
            let mut output_buf = vec![0_u8; 8192];
            let mut consumed = 0usize;

            while consumed < pcm.samples.len() {
                let info = encoder
                    .encode(&pcm.samples[consumed..], &mut output_buf)
                    .map_err(|err| TranscodeError::Encode(format!("aac encode failed: {err}")))?;

                if info.output_size > 0 {
                    output.extend_from_slice(&output_buf[..info.output_size]);
                }

                if info.input_consumed == 0 && info.output_size == 0 {
                    return Err(TranscodeError::Encode(
                        "aac encoder made no forward progress".to_string(),
                    ));
                }

                consumed += info.input_consumed;
            }

            // Flush delayed output frames.
            for _ in 0..8 {
                let info = encoder
                    .encode(&[], &mut output_buf)
                    .map_err(|err| TranscodeError::Encode(format!("aac flush failed: {err}")))?;

                if info.output_size > 0 {
                    output.extend_from_slice(&output_buf[..info.output_size]);
                }

                if info.output_size == 0 {
                    break;
                }
            }

            return Ok(output);
        }

        #[cfg(not(feature = "aac-fdk"))]
        {
            let _ = (pcm.samples.len(), pcm.sample_rate, pcm.channels, bitrate_kbps);
            Err(TranscodeError::NotImplemented(
                "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
            ))
        }
    }

    fn encode_mp3(&self, pcm: &PcmAudio, bitrate_kbps: u32) -> Result<Vec<u8>, TranscodeError> {
        let channels = match pcm.channels {
            1 | 2 => pcm.channels as u8,
            other => {
                return Err(TranscodeError::Encode(format!(
                    "unsupported channel count for MP3 encoding: {other}"
                )))
            }
        };

        let brate = mp3_bitrate_from_kbps(bitrate_kbps);
        let mode = if channels == 1 {
            Mp3Mode::Mono
        } else {
            Mp3Mode::JointStereo
        };

        let mut builder = Mp3Builder::new().ok_or_else(|| {
            TranscodeError::Encode("mp3 encoder init failed: unable to allocate encoder".to_string())
        })?;

        builder
            .set_num_channels(channels)
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;
        builder
            .set_sample_rate(pcm.sample_rate)
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;
        builder
            .set_brate(brate)
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;
        builder
            .set_mode(mode)
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;
        builder
            .set_quality(Mp3Quality::Good)
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;

        let mut encoder = builder
            .build()
            .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;

        let mut output = Vec::new();
        let chunk_samples_per_channel = 1152usize * 16;

        if channels == 1 {
            for chunk in pcm.samples.chunks(chunk_samples_per_channel) {
                output.reserve(max_required_buffer_size(chunk.len()));
                encoder
                    .encode_to_vec(MonoPcm(chunk), &mut output)
                    .map_err(|err| TranscodeError::Encode(format!("mp3 encode failed: {err}")))?;
            }
        } else {
            let chunk_samples = chunk_samples_per_channel * 2;
            for chunk in pcm.samples.chunks(chunk_samples) {
                // Interleaved stereo requires an even count of i16 values.
                let usable_len = chunk.len() - (chunk.len() % 2);
                if usable_len == 0 {
                    continue;
                }
                let usable = &chunk[..usable_len];
                output.reserve(max_required_buffer_size(usable.len() / 2));
                encoder
                    .encode_to_vec(InterleavedPcm(usable), &mut output)
                    .map_err(|err| TranscodeError::Encode(format!("mp3 encode failed: {err}")))?;
            }
        }

        output.reserve(max_required_buffer_size(0));
        encoder
            .flush_to_vec::<FlushNoGap>(&mut output)
            .map_err(|err| TranscodeError::Encode(format!("mp3 flush failed: {err}")))?;

        Ok(output)
    }
}

fn apply_metadata(
    data: Vec<u8>,
    metadata: &AudioMetadata,
    output_format: OutputFormat,
) -> Result<Vec<u8>, TranscodeError> {
    match output_format {
        // ID3 before ADTS is widely tolerated and keeps the raw AAC path lightweight.
        OutputFormat::Aac | OutputFormat::Mp3 => metadata::prepend_id3_metadata(data, metadata),
        OutputFormat::M4a => Ok(data),
    }
}

fn scale_i32_to_i16(sample: i32, source_bits_per_sample: u32) -> i16 {
    if source_bits_per_sample == 0 {
        return 0;
    }

    let scaled = if source_bits_per_sample >= 16 {
        let shift = source_bits_per_sample - 16;
        sample >> shift
    } else {
        let shift = 16 - source_bits_per_sample;
        sample << shift
    };

    scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn f32_to_i16(sample: f32) -> i16 {
    let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i32;
    scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn mp3_bitrate_from_kbps(kbps: u32) -> Mp3Bitrate {
    const OPTIONS: &[(u32, Mp3Bitrate)] = &[
        (8, Mp3Bitrate::Kbps8),
        (16, Mp3Bitrate::Kbps16),
        (24, Mp3Bitrate::Kbps24),
        (32, Mp3Bitrate::Kbps32),
        (40, Mp3Bitrate::Kbps40),
        (48, Mp3Bitrate::Kbps48),
        (64, Mp3Bitrate::Kbps64),
        (80, Mp3Bitrate::Kbps80),
        (96, Mp3Bitrate::Kbps96),
        (112, Mp3Bitrate::Kbps112),
        (128, Mp3Bitrate::Kbps128),
        (160, Mp3Bitrate::Kbps160),
        (192, Mp3Bitrate::Kbps192),
        (224, Mp3Bitrate::Kbps224),
        (256, Mp3Bitrate::Kbps256),
        (320, Mp3Bitrate::Kbps320),
    ];

    let mut best = OPTIONS[0];
    let mut best_delta = kbps.abs_diff(best.0);

    for option in OPTIONS.iter().copied().skip(1) {
        let delta = kbps.abs_diff(option.0);
        if delta < best_delta {
            best = option;
            best_delta = delta;
        }
    }

    best.1
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "aac-fdk")]
    use std::io::Cursor;

    #[cfg(feature = "aac-fdk")]
    use hound::{SampleFormat, WavSpec, WavWriter};

    #[cfg(feature = "aac-fdk")]
    use super::Transcoder;
    #[cfg(feature = "aac-fdk")]
    use crate::audio::output::OutputFormat;

    #[cfg(feature = "aac-fdk")]
    #[test]
    fn transcodes_wav_to_m4a_when_aac_feature_is_enabled() {
        let input = tiny_wav();
        let transcoder = Transcoder::new(128);
        let output = transcoder
            .transcode_with_bitrate_and_format(&input, 128, OutputFormat::M4a)
            .expect("transcode wav to m4a");

        assert_eq!(&output[4..8], b"ftyp");
        assert!(output.windows(4).any(|window| window == b"moov"));
        assert!(output.windows(4).any(|window| window == b"mdat"));
    }

    #[cfg(feature = "aac-fdk")]
    fn tiny_wav() -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let spec = WavSpec {
                channels: 2,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::new(&mut cursor, spec).expect("create wav writer");
            for _ in 0..2048 {
                writer.write_sample::<i16>(0).expect("write left");
                writer.write_sample::<i16>(0).expect("write right");
            }
            writer.finalize().expect("finalize wav");
        }
        cursor.into_inner()
    }
}
