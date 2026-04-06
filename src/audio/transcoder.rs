use std::io::Cursor;

use id3::{Tag, TagLike, Version};
use minimp3::{Decoder, Error as Mp3Error};

use crate::{
    audio::{detect, preset::QualityPreset},
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
        self.transcode_with_bitrate(input, preset.bitrate_kbps())
    }

    pub fn transcode(&self, input: &[u8]) -> Result<Vec<u8>, TranscodeError> {
        self.transcode_with_bitrate(input, self.bitrate_kbps)
    }

    pub fn transcode_with_bitrate(
        &self,
        input: &[u8],
        bitrate_kbps: u32,
    ) -> Result<Vec<u8>, TranscodeError> {
        if bitrate_kbps == 0 {
            return Err(TranscodeError::Encode("bitrate must be > 0".to_string()));
        }

        let _ = detect::detect_format(input)?;
        let artwork = extract_cover_art(input);
        let pcm = self.decode_mp3(input)?;
        let mut output = self.encode_aac(&pcm, bitrate_kbps)?;

        if let Some(picture) = artwork {
            output = prepend_id3_artwork(output, picture)?;
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
}

fn extract_cover_art(input: &[u8]) -> Option<id3::frame::Picture> {
    let mut cursor = Cursor::new(input);
    let tag = Tag::read_from2(&mut cursor).ok()?;
    let picture = tag.pictures().next().cloned();
    picture
}

fn prepend_id3_artwork(
    aac_data: Vec<u8>,
    picture: id3::frame::Picture,
) -> Result<Vec<u8>, TranscodeError> {
    let mut tag = Tag::new();
    tag.add_frame(picture);

    let mut tagged = Vec::new();
    tag.write_to(&mut tagged, Version::Id3v24)
        .map_err(|err| TranscodeError::Encode(format!("failed to write ID3 artwork tag: {err}")))?;
    tagged.extend_from_slice(&aac_data);
    Ok(tagged)
}
