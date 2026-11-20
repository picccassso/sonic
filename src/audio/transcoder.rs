use std::{
    fs::File,
    io::{BufReader, BufWriter, Cursor, Write},
    path::Path,
};

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
        opus,
        output::OutputFormat,
        pcm::PcmAudio,
        preset::QualityPreset,
    },
    errors::TranscodeError,
};
#[cfg(feature = "aac-fdk")]
use fdk_aac::enc::{AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport};

#[derive(Debug, Clone)]
pub struct Transcoder {
    bitrate_kbps: u32,
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
        let metadata = match input_format {
            InputFormat::Mp3
                if matches!(
                    output_format,
                    OutputFormat::Aac | OutputFormat::Mp3 | OutputFormat::Opus
                ) =>
            {
                metadata::extract_mp3_metadata(input)
            }
            InputFormat::Flac
                if matches!(
                    output_format,
                    OutputFormat::Aac | OutputFormat::Mp3 | OutputFormat::Opus
                ) =>
            {
                metadata::extract_flac_metadata(input)
            }
            _ => None,
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
            OutputFormat::Opus => opus::encode_opus(&pcm, bitrate_kbps, metadata.as_ref())?,
        };

        if let Some(metadata) = metadata {
            output = apply_metadata(output, &metadata, output_format)?;
        }

        Ok(output)
    }

    pub fn transcode_file_to_path(
        &self,
        input_path: &Path,
        output_path: &Path,
        bitrate_kbps: u32,
        output_format: OutputFormat,
    ) -> Result<(), TranscodeError> {
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode("bitrate must be > 0".to_string()));
        }

        let input_format = detect::detect_format_from_path(input_path)?;
        let metadata = match input_format {
            InputFormat::Mp3
                if matches!(
                    output_format,
                    OutputFormat::Aac | OutputFormat::Mp3 | OutputFormat::Opus
                ) =>
            {
                metadata::extract_mp3_metadata_from_path(input_path)
            }
            InputFormat::Flac
                if matches!(
                    output_format,
                    OutputFormat::Aac | OutputFormat::Mp3 | OutputFormat::Opus
                ) =>
            {
                metadata::extract_flac_metadata_from_path(input_path)
            }
            _ => None,
        };

        let output = File::create(output_path).map_err(|err| {
            TranscodeError::Encode(format!(
                "failed to create output file '{}': {err}",
                output_path.display()
            ))
        })?;
        let mut writer = BufWriter::new(output);

        match output_format {
            OutputFormat::Aac => {
                #[cfg(feature = "aac-fdk")]
                {
                    if let Some(meta) = metadata.as_ref() {
                        metadata::write_id3_metadata(&mut writer, meta)?;
                    }
                    let mut encoder: Option<Encoder> = None;
                    let mut out_buf = vec![0_u8; 8192];
                    stream_file_pcm(input_path, input_format, |samples, rate, ch| {
                        if encoder.is_none() {
                            encoder = Some(new_aac_encoder(rate, ch, bitrate_kbps)?);
                        }
                        encode_aac_samples_to_sink(
                            encoder.as_ref().unwrap(),
                            samples,
                            &mut out_buf,
                            &mut |adts| {
                                writer.write_all(adts).map_err(|err| {
                                    TranscodeError::Encode(format!(
                                        "aac output write failed: {err}"
                                    ))
                                })
                            },
                            "aac encode failed",
                        )
                    })?;
                    if let Some(enc) = encoder.as_ref() {
                        flush_aac_encoder(enc, &mut out_buf, &mut |adts| {
                            writer.write_all(adts).map_err(|err| {
                                TranscodeError::Encode(format!("aac output write failed: {err}"))
                            })
                        })?;
                    }
                    writer.flush().map_err(|err| {
                        TranscodeError::Encode(format!(
                            "failed to flush output file '{}': {err}",
                            output_path.display()
                        ))
                    })
                }
                #[cfg(not(feature = "aac-fdk"))]
                {
                    let _ = (writer, metadata);
                    Err(TranscodeError::NotImplemented(
                        "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
                    ))
                }
            }
            OutputFormat::M4a => {
                #[cfg(feature = "aac-fdk")]
                {
                    let mut m4a_writer = m4a::M4aWriter::new(writer, bitrate_kbps)?;
                    let mut encoder: Option<Encoder> = None;
                    let mut out_buf = vec![0_u8; 8192];
                    stream_file_pcm(input_path, input_format, |samples, rate, ch| {
                        if encoder.is_none() {
                            encoder = Some(new_aac_encoder(rate, ch, bitrate_kbps)?);
                        }
                        encode_aac_samples_to_sink(
                            encoder.as_ref().unwrap(),
                            samples,
                            &mut out_buf,
                            &mut |adts| m4a_writer.write_adts(adts),
                            "aac encode failed",
                        )
                    })?;
                    if let Some(enc) = encoder.as_ref() {
                        flush_aac_encoder(enc, &mut out_buf, &mut |adts| {
                            m4a_writer.write_adts(adts)
                        })?;
                    }
                    let mut writer = m4a_writer.finish()?;
                    writer.flush().map_err(|err| {
                        TranscodeError::Encode(format!(
                            "failed to flush output file '{}': {err}",
                            output_path.display()
                        ))
                    })
                }
                #[cfg(not(feature = "aac-fdk"))]
                {
                    let _ = (writer, metadata);
                    Err(TranscodeError::NotImplemented(
                        "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
                    ))
                }
            }
            OutputFormat::Mp3 => {
                if let Some(meta) = metadata.as_ref() {
                    metadata::write_id3_metadata(&mut writer, meta)?;
                }
                let mut encoder = None;
                let mut enc_buf = Vec::new();
                let mut ch_count = 0u16;
                stream_file_pcm(input_path, input_format, |samples, rate, ch| {
                    if encoder.is_none() {
                        ch_count = ch;
                        encoder = Some(build_mp3_encoder(rate, ch, bitrate_kbps)?);
                    }
                    let enc = encoder.as_mut().unwrap();
                    enc_buf.clear();
                    if ch_count == 1 {
                        enc_buf.reserve(max_required_buffer_size(samples.len()));
                        enc.encode_to_vec(MonoPcm(samples), &mut enc_buf)
                            .map_err(|err| {
                                TranscodeError::Encode(format!("mp3 encode failed: {err}"))
                            })?;
                    } else {
                        let usable = samples.len() - (samples.len() % 2);
                        if usable > 0 {
                            enc_buf.reserve(max_required_buffer_size(usable / 2));
                            enc.encode_to_vec(InterleavedPcm(&samples[..usable]), &mut enc_buf)
                                .map_err(|err| {
                                    TranscodeError::Encode(format!("mp3 encode failed: {err}"))
                                })?;
                        }
                    }
                    writer.write_all(&enc_buf).map_err(|err| {
                        TranscodeError::Encode(format!("mp3 output write failed: {err}"))
                    })
                })?;
                if let Some(mut enc) = encoder {
                    enc_buf.clear();
                    enc_buf.reserve(max_required_buffer_size(0));
                    enc.flush_to_vec::<FlushNoGap>(&mut enc_buf)
                        .map_err(|err| TranscodeError::Encode(format!("mp3 flush failed: {err}")))?;
                    writer.write_all(&enc_buf).map_err(|err| {
                        TranscodeError::Encode(format!("mp3 output write failed: {err}"))
                    })?;
                }
                writer.flush().map_err(|err| {
                    TranscodeError::Encode(format!(
                        "failed to flush output file '{}': {err}",
                        output_path.display()
                    ))
                })
            }
            OutputFormat::Opus => {
                let mut opus_enc: Option<opus::OpusStreamEncoder<BufWriter<File>>> = None;
                let mut writer_slot = Some(writer);
                stream_file_pcm(input_path, input_format, |samples, rate, ch| {
                    if opus_enc.is_none() {
                        let w = writer_slot.take().unwrap();
                        opus_enc = Some(opus::OpusStreamEncoder::new(
                            w,
                            rate,
                            ch,
                            bitrate_kbps,
                            metadata.as_ref(),
                        )?);
                    }
                    opus_enc.as_mut().unwrap().push(samples)
                })?;
                let enc = opus_enc.ok_or_else(|| {
                    TranscodeError::Decode("audio stream did not contain decodable samples".to_string())
                })?;
                let mut writer = enc.finish()?;
                writer.flush().map_err(|err| {
                    TranscodeError::Encode(format!(
                        "failed to flush output file '{}': {err}",
                        output_path.display()
                    ))
                })
            }
        }
    }

    pub fn transcode_file_to_opus_path(
        &self,
        input_path: &Path,
        output_path: &Path,
        bitrate_kbps: u32,
    ) -> Result<(), TranscodeError> {
        self.transcode_file_to_path(input_path, output_path, bitrate_kbps, OutputFormat::Opus)
    }

    pub fn transcode_mp3_file_to_aac_path(
        &self,
        input_path: &Path,
        output_path: &Path,
        bitrate_kbps: u32,
    ) -> Result<(), TranscodeError> {
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode("bitrate must be > 0".to_string()));
        }

        #[cfg(feature = "aac-fdk")]
        {
            let input = File::open(input_path).map_err(|err| {
                TranscodeError::Decode(format!(
                    "failed to open input file '{}': {err}",
                    input_path.display()
                ))
            })?;
            let output = File::create(output_path).map_err(|err| {
                TranscodeError::Encode(format!(
                    "failed to create output file '{}': {err}",
                    output_path.display()
                ))
            })?;

            let metadata = metadata::extract_mp3_metadata_from_path(input_path);
            let mut writer = BufWriter::new(output);
            if let Some(metadata) = metadata.as_ref() {
                metadata::write_id3_metadata(&mut writer, metadata)?;
            }

            self.transcode_mp3_reader_to_aac_writer(
                BufReader::new(input),
                &mut writer,
                bitrate_kbps,
            )?;
            writer.flush().map_err(|err| {
                TranscodeError::Encode(format!(
                    "failed to flush output file '{}': {err}",
                    output_path.display()
                ))
            })?;
            return Ok(());
        }

        #[cfg(not(feature = "aac-fdk"))]
        {
            let _ = (input_path, output_path, bitrate_kbps);
            Err(TranscodeError::NotImplemented(
                "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
            ))
        }
    }

    pub fn transcode_mp3_file_to_m4a_path(
        &self,
        input_path: &Path,
        output_path: &Path,
        bitrate_kbps: u32,
    ) -> Result<(), TranscodeError> {
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode("bitrate must be > 0".to_string()));
        }

        #[cfg(feature = "aac-fdk")]
        {
            let input = File::open(input_path).map_err(|err| {
                TranscodeError::Decode(format!(
                    "failed to open input file '{}': {err}",
                    input_path.display()
                ))
            })?;
            let output = File::create(output_path).map_err(|err| {
                TranscodeError::Encode(format!(
                    "failed to create output file '{}': {err}",
                    output_path.display()
                ))
            })?;

            let mut writer = m4a::M4aWriter::new(BufWriter::new(output), bitrate_kbps)?;
            self.transcode_mp3_reader_to_aac_sink(
                BufReader::new(input),
                |adts| writer.write_adts(adts),
                bitrate_kbps,
            )?;
            let mut writer = writer.finish()?;
            writer.flush().map_err(|err| {
                TranscodeError::Encode(format!(
                    "failed to flush output file '{}': {err}",
                    output_path.display()
                ))
            })?;
            return Ok(());
        }

        #[cfg(not(feature = "aac-fdk"))]
        {
            let _ = (input_path, output_path, bitrate_kbps);
            Err(TranscodeError::NotImplemented(
                "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
            ))
        }
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
                Err(err) => {
                    return Err(TranscodeError::Decode(format!("mp3 decode failed: {err}")))
                }
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
                            s.map(|sample| scale_i32_to_i16(sample as i32, bits))
                                .map_err(|err| {
                                    TranscodeError::Decode(format!(
                                        "wav sample decode failed: {err}"
                                    ))
                                })
                        })
                        .collect()
                } else {
                    reader
                        .samples::<i32>()
                        .map(|s| {
                            s.map(|sample| scale_i32_to_i16(sample, bits))
                                .map_err(|err| {
                                    TranscodeError::Decode(format!(
                                        "wav sample decode failed: {err}"
                                    ))
                                })
                        })
                        .collect()
                }
            }
            WavSampleFormat::Float => reader
                .samples::<f32>()
                .map(|s| {
                    s.map(f32_to_i16).map_err(|err| {
                        TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                    })
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
            let sample = sample.map_err(|err| {
                TranscodeError::Decode(format!("flac sample decode failed: {err}"))
            })?;
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

            let mut output = Vec::with_capacity(estimated_aac_output_capacity(
                pcm.samples.len(),
                pcm.sample_rate,
                pcm.channels,
                bitrate_kbps,
            ));
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
            let _ = (
                pcm.samples.len(),
                pcm.sample_rate,
                pcm.channels,
                bitrate_kbps,
            );
            Err(TranscodeError::NotImplemented(
                "aac encoder unavailable; rebuild with --features aac-fdk".to_string(),
            ))
        }
    }

    #[cfg(feature = "aac-fdk")]
    fn transcode_mp3_reader_to_aac_writer<R: std::io::Read, W: Write>(
        &self,
        reader: R,
        writer: &mut W,
        bitrate_kbps: u32,
    ) -> Result<(), TranscodeError> {
        self.transcode_mp3_reader_to_aac_sink(
            reader,
            |adts| {
                writer.write_all(adts).map_err(|err| {
                    TranscodeError::Encode(format!("aac output write failed: {err}"))
                })
            },
            bitrate_kbps,
        )
    }

    #[cfg(feature = "aac-fdk")]
    fn transcode_mp3_reader_to_aac_sink<R, F>(
        &self,
        reader: R,
        mut sink: F,
        bitrate_kbps: u32,
    ) -> Result<(), TranscodeError>
    where
        R: std::io::Read,
        F: FnMut(&[u8]) -> Result<(), TranscodeError>,
    {
        let mut decoder = Decoder::new(reader);
        let mut encoder = None;
        let mut sample_rate = None;
        let mut channels = None;
        let mut output_buf = vec![0_u8; 8192];
        let mut decoded_frames = 0usize;

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

                    if encoder.is_none() {
                        encoder = Some(new_aac_encoder(frame_rate, frame_channels, bitrate_kbps)?);
                    }

                    if let Some(encoder) = encoder.as_ref() {
                        encode_aac_samples_to_sink(
                            encoder,
                            &frame.data,
                            &mut output_buf,
                            &mut sink,
                            "aac encode failed",
                        )?;
                    }
                    decoded_frames += 1;
                }
                Err(Mp3Error::Eof) => break,
                Err(err) => {
                    return Err(TranscodeError::Decode(format!("mp3 decode failed: {err}")))
                }
            }
        }

        if decoded_frames == 0 {
            return Err(TranscodeError::Decode(
                "mp3 stream did not contain decodable frames".to_string(),
            ));
        }

        if let Some(encoder) = encoder.as_ref() {
            for _ in 0..8 {
                let info = encoder
                    .encode(&[], &mut output_buf)
                    .map_err(|err| TranscodeError::Encode(format!("aac flush failed: {err}")))?;

                if info.output_size > 0 {
                    sink(&output_buf[..info.output_size])?;
                }

                if info.output_size == 0 {
                    break;
                }
            }
        }

        Ok(())
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
            TranscodeError::Encode(
                "mp3 encoder init failed: unable to allocate encoder".to_string(),
            )
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
        OutputFormat::M4a | OutputFormat::Opus => Ok(data),
    }
}

#[cfg(feature = "aac-fdk")]
fn new_aac_encoder(
    sample_rate: u32,
    channels: u16,
    bitrate_kbps: u32,
) -> Result<Encoder, TranscodeError> {
    let channels = match channels {
        1 => ChannelMode::Mono,
        2 => ChannelMode::Stereo,
        other => {
            return Err(TranscodeError::Encode(format!(
                "unsupported channel count for AAC encoding: {other}"
            )))
        }
    };

    Encoder::new(EncoderParams {
        bit_rate: BitRate::Cbr(bitrate_kbps.saturating_mul(1000)),
        sample_rate,
        transport: Transport::Adts,
        channels,
        audio_object_type: AudioObjectType::Mpeg4LowComplexity,
    })
    .map_err(|err| TranscodeError::Encode(format!("aac encoder init failed: {err}")))
}

#[cfg(feature = "aac-fdk")]
fn encode_aac_samples_to_sink(
    encoder: &Encoder,
    samples: &[i16],
    output_buf: &mut [u8],
    sink: &mut impl FnMut(&[u8]) -> Result<(), TranscodeError>,
    error_context: &str,
) -> Result<(), TranscodeError> {
    let mut consumed = 0usize;

    while consumed < samples.len() {
        let info = encoder
            .encode(&samples[consumed..], output_buf)
            .map_err(|err| TranscodeError::Encode(format!("{error_context}: {err}")))?;

        if info.output_size > 0 {
            sink(&output_buf[..info.output_size])?;
        }

        if info.input_consumed == 0 && info.output_size == 0 {
            return Err(TranscodeError::Encode(
                "aac encoder made no forward progress".to_string(),
            ));
        }

        consumed += info.input_consumed;
    }

    Ok(())
}

#[cfg(feature = "aac-fdk")]
fn estimated_aac_output_capacity(
    samples_len: usize,
    sample_rate: u32,
    channels: u16,
    bitrate_kbps: u32,
) -> usize {
    if sample_rate == 0 || channels == 0 || bitrate_kbps == 0 {
        return 16 * 1024;
    }

    let samples_per_channel = samples_len as u64 / u64::from(channels);
    let bitrate_bps = u64::from(bitrate_kbps).saturating_mul(1000);
    let encoded_bytes = samples_per_channel
        .saturating_mul(bitrate_bps)
        .saturating_div(u64::from(sample_rate))
        .saturating_div(8);

    encoded_bytes
        .saturating_add(32 * 1024)
        .min(usize::MAX as u64) as usize
}

#[cfg(feature = "aac-fdk")]
fn flush_aac_encoder(
    encoder: &Encoder,
    output_buf: &mut [u8],
    sink: &mut impl FnMut(&[u8]) -> Result<(), TranscodeError>,
) -> Result<(), TranscodeError> {
    for _ in 0..8 {
        let info = encoder
            .encode(&[], output_buf)
            .map_err(|err| TranscodeError::Encode(format!("aac flush failed: {err}")))?;
        if info.output_size > 0 {
            sink(&output_buf[..info.output_size])?;
        }
        if info.output_size == 0 {
            break;
        }
    }
    Ok(())
}

fn build_mp3_encoder(
    sample_rate: u32,
    channels: u16,
    bitrate_kbps: u32,
) -> Result<mp3lame_encoder::Encoder, TranscodeError> {
    let ch = match channels {
        1 | 2 => channels as u8,
        other => {
            return Err(TranscodeError::Encode(format!(
                "unsupported channel count for MP3 encoding: {other}"
            )))
        }
    };
    let brate = mp3_bitrate_from_kbps(bitrate_kbps);
    let mode = if ch == 1 {
        Mp3Mode::Mono
    } else {
        Mp3Mode::JointStereo
    };
    let mut builder = Mp3Builder::new().ok_or_else(|| {
        TranscodeError::Encode("mp3 encoder init failed: unable to allocate encoder".to_string())
    })?;
    builder
        .set_num_channels(ch)
        .and_then(|_| builder.set_sample_rate(sample_rate))
        .and_then(|_| builder.set_brate(brate))
        .and_then(|_| builder.set_mode(mode))
        .and_then(|_| builder.set_quality(Mp3Quality::Good))
        .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))?;
    builder
        .build()
        .map_err(|err| TranscodeError::Encode(format!("mp3 encoder init failed: {err}")))
}

fn stream_file_pcm(
    input_path: &Path,
    format: InputFormat,
    mut on_chunk: impl FnMut(&[i16], u32, u16) -> Result<(), TranscodeError>,
) -> Result<(), TranscodeError> {
    const WAV_CHUNK_FRAMES: usize = 4096;

    match format {
        InputFormat::Mp3 => {
            let input = File::open(input_path).map_err(|err| {
                TranscodeError::Decode(format!(
                    "failed to open input file '{}': {err}",
                    input_path.display()
                ))
            })?;
            let mut decoder = Decoder::new(BufReader::new(input));
            let mut sample_rate = None;
            let mut channels = None;
            let mut decoded_frames = 0usize;

            loop {
                match decoder.next_frame() {
                    Ok(frame) => {
                        let rate = frame.sample_rate as u32;
                        let ch = frame.channels as u16;
                        sample_rate.get_or_insert(rate);
                        channels.get_or_insert(ch);
                        if sample_rate != Some(rate) || channels != Some(ch) {
                            return Err(TranscodeError::Decode(
                                "mp3 stream changed sample rate or channels mid-stream".to_string(),
                            ));
                        }
                        if !frame.data.is_empty() {
                            on_chunk(&frame.data, rate, ch)?;
                            decoded_frames += 1;
                        }
                    }
                    Err(Mp3Error::Eof) => break,
                    Err(err) => {
                        return Err(TranscodeError::Decode(format!("mp3 decode failed: {err}")))
                    }
                }
            }
            if decoded_frames == 0 {
                return Err(TranscodeError::Decode(
                    "mp3 stream did not contain decodable frames".to_string(),
                ));
            }
            Ok(())
        }
        InputFormat::Flac => {
            let mut reader = FlacReader::open(input_path)
                .map_err(|err| TranscodeError::Decode(format!("flac decode failed: {err}")))?;
            let info = reader.streaminfo();
            if info.channels == 0 || info.channels > 2 {
                return Err(TranscodeError::Decode(format!(
                    "unsupported flac channel count: {} (expected mono/stereo)",
                    info.channels
                )));
            }
            let rate = info.sample_rate;
            let ch = info.channels as u16;
            let bits = u32::from(info.bits_per_sample);

            let mut block_buf = Vec::new();
            let mut interleaved = Vec::new();
            let mut blocks = reader.blocks();
            let mut total_samples = 0usize;

            while let Some(block) = blocks
                .read_next_or_eof(block_buf)
                .map_err(|err| TranscodeError::Decode(format!("flac sample decode failed: {err}")))?
            {
                let frames = block.duration() as usize;
                interleaved.clear();
                interleaved.reserve(frames * usize::from(ch));
                for frame in 0..frames {
                    for c in 0..u32::from(ch) {
                        interleaved.push(scale_i32_to_i16(block.sample(c, frame as u32), bits));
                    }
                }
                if !interleaved.is_empty() {
                    total_samples += interleaved.len();
                    on_chunk(&interleaved, rate, ch)?;
                }
                block_buf = block.into_buffer();
            }
            if total_samples == 0 {
                return Err(TranscodeError::Decode(
                    "flac stream did not contain decodable samples".to_string(),
                ));
            }
            Ok(())
        }
        InputFormat::Wav => {
            let mut reader = WavReader::open(input_path)
                .map_err(|err| TranscodeError::Decode(format!("wav decode failed: {err}")))?;
            let spec = reader.spec();
            if spec.channels == 0 || spec.channels > 2 {
                return Err(TranscodeError::Decode(format!(
                    "unsupported wav channel count: {} (expected mono/stereo)",
                    spec.channels
                )));
            }
            let rate = spec.sample_rate;
            let ch = spec.channels;
            let bits = spec.bits_per_sample as u32;
            let chunk_cap = WAV_CHUNK_FRAMES * usize::from(ch);
            let mut chunk = Vec::with_capacity(chunk_cap);
            let mut total_samples = 0usize;

            match spec.sample_format {
                WavSampleFormat::Int if bits <= 16 => {
                    for sample in reader.samples::<i16>() {
                        let s = sample.map_err(|err| {
                            TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                        })?;
                        chunk.push(scale_i32_to_i16(s as i32, bits));
                        if chunk.len() >= chunk_cap {
                            total_samples += chunk.len();
                            on_chunk(&chunk, rate, ch)?;
                            chunk.clear();
                        }
                    }
                }
                WavSampleFormat::Int => {
                    for sample in reader.samples::<i32>() {
                        let s = sample.map_err(|err| {
                            TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                        })?;
                        chunk.push(scale_i32_to_i16(s, bits));
                        if chunk.len() >= chunk_cap {
                            total_samples += chunk.len();
                            on_chunk(&chunk, rate, ch)?;
                            chunk.clear();
                        }
                    }
                }
                WavSampleFormat::Float => {
                    for sample in reader.samples::<f32>() {
                        let s = sample.map_err(|err| {
                            TranscodeError::Decode(format!("wav sample decode failed: {err}"))
                        })?;
                        chunk.push(f32_to_i16(s));
                        if chunk.len() >= chunk_cap {
                            total_samples += chunk.len();
                            on_chunk(&chunk, rate, ch)?;
                            chunk.clear();
                        }
                    }
                }
            }

            if !chunk.is_empty() {
                total_samples += chunk.len();
                on_chunk(&chunk, rate, ch)?;
            }
            if total_samples == 0 {
                return Err(TranscodeError::Decode(
                    "wav stream did not contain decodable samples".to_string(),
                ));
            }
            Ok(())
        }
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
    use std::{
        fs,
        io::Cursor,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use hound::{SampleFormat, WavSpec, WavWriter};

    use super::Transcoder;
    use crate::audio::{output::OutputFormat, preset::QualityPreset};

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
            for i in 0..2048 {
                let sample = ((i as f32 * 0.1).sin() * 10000.0) as i16;
                writer.write_sample::<i16>(sample).expect("write left");
                writer.write_sample::<i16>(-sample).expect("write right");
            }
            writer.finalize().expect("finalize wav");
        }
        cursor.into_inner()
    }

    #[test]
    fn transcodes_wav_to_opus_at_all_presets() {
        let input = tiny_wav();
        let transcoder = Transcoder::new(192);

        for preset in [
            QualityPreset::Low,
            QualityPreset::Medium,
            QualityPreset::High,
            QualityPreset::VeryHigh,
        ] {
            let output = transcoder
                .transcode_with_preset_and_format(&input, preset, OutputFormat::Opus)
                .expect("transcode wav to opus with preset");

            assert!(!output.is_empty());
            assert_eq!(&output[..4], b"OggS");
        }
    }

    #[test]
    fn transcodes_file_to_opus_file() {
        let input_wav = tiny_wav();
        let transcoder = Transcoder::new(192);

        let root = temp_dir("sonic-opus-file-test");
        fs::create_dir_all(&root).expect("create temp dir");
        let input_path = root.join("input.wav");
        let output_path = root.join("output.opus");
        fs::write(&input_path, input_wav).expect("write wav input");

        transcoder
            .transcode_file_to_opus_path(&input_path, &output_path, 192)
            .expect("transcode file to opus");

        let output = fs::read(&output_path).expect("read opus output");
        assert_eq!(&output[..4], b"OggS");

        let _ = fs::remove_dir_all(root);
    }

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
    #[test]
    fn streams_mp3_file_to_aac_file_when_aac_feature_is_enabled() {
        let input_wav = tiny_wav();
        let transcoder = Transcoder::new(128);
        let mp3 = transcoder
            .transcode_with_bitrate_and_format(&input_wav, 128, OutputFormat::Mp3)
            .expect("create mp3 fixture");

        let root = temp_dir("sonic-streaming-mp3-aac-test");
        fs::create_dir_all(&root).expect("create temp dir");
        let input_path = root.join("input.mp3");
        let output_path = root.join("output.aac");
        fs::write(&input_path, mp3).expect("write mp3 input");

        transcoder
            .transcode_mp3_file_to_aac_path(&input_path, &output_path, 64)
            .expect("stream mp3 to aac");

        let output = fs::read(&output_path).expect("read aac output");
        assert!(output.len() > 7);
        assert_eq!(output[0], 0xFF);
        assert_eq!(output[1] & 0xF0, 0xF0);

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(feature = "aac-fdk")]
    #[test]
    fn streams_mp3_file_to_m4a_file_when_aac_feature_is_enabled() {
        let input_wav = tiny_wav();
        let transcoder = Transcoder::new(128);
        let mp3 = transcoder
            .transcode_with_bitrate_and_format(&input_wav, 128, OutputFormat::Mp3)
            .expect("create mp3 fixture");

        let root = temp_dir("sonic-streaming-mp3-m4a-test");
        fs::create_dir_all(&root).expect("create temp dir");
        let input_path = root.join("input.mp3");
        let output_path = root.join("output.m4a");
        fs::write(&input_path, mp3).expect("write mp3 input");

        transcoder
            .transcode_mp3_file_to_m4a_path(&input_path, &output_path, 64)
            .expect("stream mp3 to m4a");

        let output = fs::read(&output_path).expect("read m4a output");
        assert_eq!(&output[4..8], b"ftyp");
        assert!(output.windows(4).any(|window| window == b"moov"));
        assert!(output.windows(4).any(|window| window == b"mdat"));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{id}"))
    }
}
