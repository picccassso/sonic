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
    let metadata = metadata::extract_mp3_metadata(input);
    if let Some(info) = probe_mp3_from_headers(input, metadata.as_ref()) {
        return Ok(info);
    }

    let mut decoder = Decoder::new(Cursor::new(input));
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

#[derive(Debug, Clone, Copy)]
struct Mp3FrameHeader {
    sample_rate: u32,
    channels: u16,
    bitrate_kbps: u16,
    samples_per_frame: u16,
    mpeg1: bool,
}

fn probe_mp3_from_headers(
    input: &[u8],
    metadata: Option<&metadata::AudioMetadata>,
) -> Option<AudioInfo> {
    let start = find_first_mp3_frame(input)?;
    let header = parse_mp3_frame_header(input.get(start..start + 4)?)?;
    let audio_len = input
        .len()
        .saturating_sub(start)
        .saturating_sub(id3v1_len(input));

    let total_samples = read_xing_frame_count(input, start, header)
        .or_else(|| read_vbri_frame_count(input, start).map(u64::from))
        .map(|frames| frames.saturating_mul(u64::from(header.samples_per_frame)))
        .unwrap_or_else(|| {
            let duration_ms = (audio_len as u64).saturating_mul(8).saturating_mul(1000)
                / u64::from(header.bitrate_kbps)
                / 1000;
            duration_ms.saturating_mul(u64::from(header.sample_rate)) / 1000
        });

    if total_samples == 0 {
        return None;
    }

    Some(AudioInfo {
        input_format: InputFormat::Mp3,
        sample_rate: header.sample_rate,
        channels: header.channels,
        bits_per_sample: 16,
        duration_ms: samples_to_ms(total_samples, header.sample_rate),
        total_samples_per_channel: total_samples,
        has_metadata: metadata.map(|m| !m.is_empty()).unwrap_or(false),
        has_artwork: metadata.and_then(|m| m.artwork.as_ref()).is_some(),
    })
}

fn find_first_mp3_frame(input: &[u8]) -> Option<usize> {
    let mut pos = id3v2_len(input).unwrap_or(0);
    let scan_end = input.len().min(pos.saturating_add(64 * 1024));

    while pos + 4 <= scan_end {
        if parse_mp3_frame_header(&input[pos..pos + 4]).is_some() {
            return Some(pos);
        }
        pos += 1;
    }

    None
}

fn id3v2_len(input: &[u8]) -> Option<usize> {
    if input.len() < 10 || !input.starts_with(b"ID3") {
        return None;
    }

    let size = ((usize::from(input[6] & 0x7F)) << 21)
        | ((usize::from(input[7] & 0x7F)) << 14)
        | ((usize::from(input[8] & 0x7F)) << 7)
        | usize::from(input[9] & 0x7F);
    Some(10 + size)
}

fn id3v1_len(input: &[u8]) -> usize {
    if input.len() >= 128 && &input[input.len() - 128..input.len() - 125] == b"TAG" {
        128
    } else {
        0
    }
}

fn parse_mp3_frame_header(header: &[u8]) -> Option<Mp3FrameHeader> {
    if header.len() < 4 || header[0] != 0xFF || (header[1] & 0xE0) != 0xE0 {
        return None;
    }

    let version_bits = (header[1] >> 3) & 0x03;
    let layer_bits = (header[1] >> 1) & 0x03;
    let bitrate_index = (header[2] >> 4) & 0x0F;
    let sample_rate_index = (header[2] >> 2) & 0x03;
    let padding = usize::from((header[2] >> 1) & 0x01);
    let channel_mode = (header[3] >> 6) & 0x03;

    if version_bits == 0x01
        || layer_bits == 0
        || bitrate_index == 0
        || bitrate_index == 0x0F
        || sample_rate_index == 0x03
    {
        return None;
    }

    let mpeg1 = version_bits == 0x03;
    let sample_rate = sample_rate_from_header(version_bits, sample_rate_index)?;
    let bitrate_kbps = bitrate_from_header(version_bits, layer_bits, bitrate_index)?;
    let channels = if channel_mode == 0x03 { 1 } else { 2 };
    let samples_per_frame = samples_per_mp3_frame(mpeg1, layer_bits)?;
    let bitrate = usize::from(bitrate_kbps) * 1000;
    let frame_len = match layer_bits {
        0x03 => ((12 * bitrate / sample_rate as usize) + padding) * 4,
        0x02 => (144 * bitrate / sample_rate as usize) + padding,
        0x01 if mpeg1 => (144 * bitrate / sample_rate as usize) + padding,
        0x01 => (72 * bitrate / sample_rate as usize) + padding,
        _ => return None,
    };

    if frame_len < 4 {
        return None;
    }

    Some(Mp3FrameHeader {
        sample_rate,
        channels,
        bitrate_kbps,
        samples_per_frame,
        mpeg1,
    })
}

fn sample_rate_from_header(version_bits: u8, sample_rate_index: u8) -> Option<u32> {
    let base = match sample_rate_index {
        0 => 44_100,
        1 => 48_000,
        2 => 32_000,
        _ => return None,
    };

    match version_bits {
        0x03 => Some(base),
        0x02 => Some(base / 2),
        0x00 => Some(base / 4),
        _ => None,
    }
}

fn bitrate_from_header(version_bits: u8, layer_bits: u8, index: u8) -> Option<u16> {
    const MPEG1_LAYER1: [u16; 16] = [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
    ];
    const MPEG1_LAYER2: [u16; 16] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
    ];
    const MPEG1_LAYER3: [u16; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG2_LAYER1: [u16; 16] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
    ];
    const MPEG2_LAYER23: [u16; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];

    let table = match (version_bits == 0x03, layer_bits) {
        (true, 0x03) => &MPEG1_LAYER1,
        (true, 0x02) => &MPEG1_LAYER2,
        (true, 0x01) => &MPEG1_LAYER3,
        (false, 0x03) => &MPEG2_LAYER1,
        (false, 0x02 | 0x01) => &MPEG2_LAYER23,
        _ => return None,
    };
    table
        .get(index as usize)
        .copied()
        .filter(|value| *value > 0)
}

fn samples_per_mp3_frame(mpeg1: bool, layer_bits: u8) -> Option<u16> {
    match layer_bits {
        0x03 => Some(384),
        0x02 => Some(1152),
        0x01 if mpeg1 => Some(1152),
        0x01 => Some(576),
        _ => None,
    }
}

fn read_xing_frame_count(input: &[u8], frame_start: usize, header: Mp3FrameHeader) -> Option<u64> {
    let side_info_len = match (header.mpeg1, header.channels) {
        (true, 1) => 17,
        (true, _) => 32,
        (false, 1) => 9,
        (false, _) => 17,
    };
    let pos = frame_start + 4 + side_info_len;
    let tag = input.get(pos..pos + 16)?;
    if &tag[0..4] != b"Xing" && &tag[0..4] != b"Info" {
        return None;
    }

    let flags = u32::from_be_bytes(tag[4..8].try_into().ok()?);
    if flags & 0x01 == 0 {
        return None;
    }
    Some(u64::from(u32::from_be_bytes(tag[8..12].try_into().ok()?)))
}

fn read_vbri_frame_count(input: &[u8], frame_start: usize) -> Option<u32> {
    let pos = frame_start + 4 + 32;
    let tag = input.get(pos..pos + 26)?;
    if &tag[0..4] != b"VBRI" {
        return None;
    }
    Some(u32::from_be_bytes(tag[14..18].try_into().ok()?))
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
