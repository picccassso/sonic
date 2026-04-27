use crate::errors::TranscodeError;

#[derive(Debug, Clone)]
struct AacFrame {
    data_offset: usize,
    data_len: usize,
}

#[derive(Debug, Clone)]
struct AacConfig {
    audio_object_type: u8,
    sample_rate_index: u8,
    sample_rate: u32,
    channel_config: u8,
    frames: Vec<AacFrame>,
}

pub fn adts_to_m4a(adts: &[u8], bitrate_kbps: u32) -> Result<Vec<u8>, TranscodeError> {
    let config = parse_adts(adts)?;
    let mut aac_payload = Vec::new();
    let mut sample_sizes = Vec::with_capacity(config.frames.len());

    for frame in &config.frames {
        aac_payload.extend_from_slice(&adts[frame.data_offset..frame.data_offset + frame.data_len]);
        sample_sizes.push(frame.data_len as u32);
    }

    let ftyp = atom(b"ftyp", &ftyp_payload());
    let mdat_header_len = 8usize;
    let first_sample_offset = (ftyp.len() + mdat_header_len) as u32;
    let mdat = atom(b"mdat", &aac_payload);
    let moov = moov_payload(&config, &sample_sizes, first_sample_offset, bitrate_kbps);

    let mut out = Vec::with_capacity(ftyp.len() + mdat.len() + moov.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&mdat);
    out.extend_from_slice(&moov);
    Ok(out)
}

fn parse_adts(input: &[u8]) -> Result<AacConfig, TranscodeError> {
    let mut pos = 0usize;
    let mut frames = Vec::new();
    let mut audio_object_type = None;
    let mut sample_rate_index = None;
    let mut channel_config = None;

    while pos + 7 <= input.len() {
        if input[pos] != 0xFF || (input[pos + 1] & 0xF0) != 0xF0 {
            return Err(TranscodeError::Encode(
                "aac output was not valid ADTS and cannot be wrapped as M4A".to_string(),
            ));
        }

        let protection_absent = input[pos + 1] & 0x01;
        let header_len = if protection_absent == 1 { 7 } else { 9 };
        if pos + header_len > input.len() {
            return Err(TranscodeError::Encode("truncated ADTS header".to_string()));
        }

        let profile = (input[pos + 2] & 0xC0) >> 6;
        let sf_index = (input[pos + 2] & 0x3C) >> 2;
        let chan_config = ((input[pos + 2] & 0x01) << 2) | ((input[pos + 3] & 0xC0) >> 6);
        let frame_len = (((input[pos + 3] & 0x03) as usize) << 11)
            | ((input[pos + 4] as usize) << 3)
            | (((input[pos + 5] & 0xE0) as usize) >> 5);

        if frame_len < header_len || pos + frame_len > input.len() {
            return Err(TranscodeError::Encode("invalid ADTS frame length".to_string()));
        }

        audio_object_type.get_or_insert(profile + 1);
        sample_rate_index.get_or_insert(sf_index);
        channel_config.get_or_insert(chan_config);

        if audio_object_type != Some(profile + 1)
            || sample_rate_index != Some(sf_index)
            || channel_config != Some(chan_config)
        {
            return Err(TranscodeError::Encode(
                "AAC stream changed configuration mid-stream".to_string(),
            ));
        }

        frames.push(AacFrame {
            data_offset: pos + header_len,
            data_len: frame_len - header_len,
        });
        pos += frame_len;
    }

    if pos != input.len() || frames.is_empty() {
        return Err(TranscodeError::Encode("invalid or empty ADTS stream".to_string()));
    }

    let sample_rate_index = sample_rate_index.unwrap_or_default();
    let sample_rate = sample_rate_from_index(sample_rate_index).ok_or_else(|| {
        TranscodeError::Encode(format!("unsupported AAC sample-rate index: {sample_rate_index}"))
    })?;

    Ok(AacConfig {
        audio_object_type: audio_object_type.unwrap_or(2),
        sample_rate_index,
        sample_rate,
        channel_config: channel_config.unwrap_or(2),
        frames,
    })
}

fn ftyp_payload() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"M4A ");
    write_u32(&mut out, 0);
    out.extend_from_slice(b"M4A ");
    out.extend_from_slice(b"mp42");
    out.extend_from_slice(b"isom");
    out
}

fn moov_payload(
    config: &AacConfig,
    sample_sizes: &[u32],
    first_sample_offset: u32,
    bitrate_kbps: u32,
) -> Vec<u8> {
    let sample_count = sample_sizes.len() as u32;
    let media_duration = sample_count.saturating_mul(1024);
    let movie_duration = (u64::from(media_duration) * 1000 / u64::from(config.sample_rate)) as u32;

    atom(
        b"moov",
        &[
            mvhd_payload(movie_duration),
            trak_payload(
                config,
                sample_sizes,
                first_sample_offset,
                bitrate_kbps,
                media_duration,
                movie_duration,
            ),
        ]
        .concat(),
    )
}

fn mvhd_payload(duration: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 1000);
    write_u32(&mut out, duration);
    write_u32(&mut out, 0x0001_0000);
    write_u16(&mut out, 0x0100);
    write_u16(&mut out, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_matrix(&mut out);
    for _ in 0..6 {
        write_u32(&mut out, 0);
    }
    write_u32(&mut out, 2);
    atom(b"mvhd", &out)
}

fn trak_payload(
    config: &AacConfig,
    sample_sizes: &[u32],
    first_sample_offset: u32,
    bitrate_kbps: u32,
    media_duration: u32,
    movie_duration: u32,
) -> Vec<u8> {
    atom(
        b"trak",
        &[
            tkhd_payload(movie_duration),
            mdia_payload(
                config,
                sample_sizes,
                first_sample_offset,
                bitrate_kbps,
                media_duration,
            ),
        ]
        .concat(),
    )
}

fn tkhd_payload(duration: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0x000007);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 1);
    write_u32(&mut out, 0);
    write_u32(&mut out, duration);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0x0100);
    write_u16(&mut out, 0);
    write_matrix(&mut out);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    atom(b"tkhd", &out)
}

fn mdia_payload(
    config: &AacConfig,
    sample_sizes: &[u32],
    first_sample_offset: u32,
    bitrate_kbps: u32,
    media_duration: u32,
) -> Vec<u8> {
    atom(
        b"mdia",
        &[
            mdhd_payload(config.sample_rate, media_duration),
            hdlr_payload(),
            minf_payload(config, sample_sizes, first_sample_offset, bitrate_kbps),
        ]
        .concat(),
    )
}

fn mdhd_payload(sample_rate: u32, duration: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, sample_rate);
    write_u32(&mut out, duration);
    write_u16(&mut out, 0x55C4);
    write_u16(&mut out, 0);
    atom(b"mdhd", &out)
}

fn hdlr_payload() -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 0);
    out.extend_from_slice(b"soun");
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, 0);
    out.extend_from_slice(b"Sonic Audio\0");
    atom(b"hdlr", &out)
}

fn minf_payload(
    config: &AacConfig,
    sample_sizes: &[u32],
    first_sample_offset: u32,
    bitrate_kbps: u32,
) -> Vec<u8> {
    atom(
        b"minf",
        &[
            smhd_payload(),
            dinf_payload(),
            stbl_payload(config, sample_sizes, first_sample_offset, bitrate_kbps),
        ]
        .concat(),
    )
}

fn smhd_payload() -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0);
    atom(b"smhd", &out)
}

fn dinf_payload() -> Vec<u8> {
    let mut dref = full_atom_payload(0, 0);
    write_u32(&mut dref, 1);
    dref.extend_from_slice(&atom(b"url ", &full_atom_payload(0, 1)));
    atom(b"dinf", &atom(b"dref", &dref))
}

fn stbl_payload(
    config: &AacConfig,
    sample_sizes: &[u32],
    first_sample_offset: u32,
    bitrate_kbps: u32,
) -> Vec<u8> {
    atom(
        b"stbl",
        &[
            stsd_payload(config, bitrate_kbps),
            stts_payload(sample_sizes.len() as u32),
            stsc_payload(sample_sizes.len() as u32),
            stsz_payload(sample_sizes),
            stco_payload(first_sample_offset),
        ]
        .concat(),
    )
}

fn stsd_payload(config: &AacConfig, bitrate_kbps: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 1);

    let mut mp4a = Vec::new();
    mp4a.extend_from_slice(&[0; 6]);
    write_u16(&mut mp4a, 1);
    write_u32(&mut mp4a, 0);
    write_u32(&mut mp4a, 0);
    write_u16(&mut mp4a, config.channel_config as u16);
    write_u16(&mut mp4a, 16);
    write_u16(&mut mp4a, 0);
    write_u16(&mut mp4a, 0);
    write_u32(&mut mp4a, config.sample_rate << 16);
    mp4a.extend_from_slice(&esds_payload(config, bitrate_kbps));

    out.extend_from_slice(&atom(b"mp4a", &mp4a));
    atom(b"stsd", &out)
}

fn esds_payload(config: &AacConfig, bitrate_kbps: u32) -> Vec<u8> {
    let asc = [
        (config.audio_object_type << 3) | (config.sample_rate_index >> 1),
        ((config.sample_rate_index & 0x01) << 7) | (config.channel_config << 3),
    ];

    let mut dec_specific = Vec::new();
    write_descriptor(&mut dec_specific, 0x05, &asc);

    let mut dec_config = Vec::new();
    dec_config.push(0x40);
    dec_config.push(0x15);
    dec_config.extend_from_slice(&[0, 0, 0]);
    let bitrate = bitrate_kbps.saturating_mul(1000);
    write_u32(&mut dec_config, bitrate);
    write_u32(&mut dec_config, bitrate);
    dec_config.extend_from_slice(&dec_specific);

    let mut sl_config = Vec::new();
    write_descriptor(&mut sl_config, 0x06, &[0x02]);

    let mut es = Vec::new();
    write_u16(&mut es, 1);
    es.push(0);
    write_descriptor(&mut es, 0x04, &dec_config);
    es.extend_from_slice(&sl_config);

    let mut out = full_atom_payload(0, 0);
    write_descriptor(&mut out, 0x03, &es);
    atom(b"esds", &out)
}

fn stts_payload(sample_count: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 1);
    write_u32(&mut out, sample_count);
    write_u32(&mut out, 1024);
    atom(b"stts", &out)
}

fn stsc_payload(sample_count: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 1);
    write_u32(&mut out, 1);
    write_u32(&mut out, sample_count);
    write_u32(&mut out, 1);
    atom(b"stsc", &out)
}

fn stsz_payload(sample_sizes: &[u32]) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 0);
    write_u32(&mut out, sample_sizes.len() as u32);
    for size in sample_sizes {
        write_u32(&mut out, *size);
    }
    atom(b"stsz", &out)
}

fn stco_payload(first_sample_offset: u32) -> Vec<u8> {
    let mut out = full_atom_payload(0, 0);
    write_u32(&mut out, 1);
    write_u32(&mut out, first_sample_offset);
    atom(b"stco", &out)
}

fn atom(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    write_u32(&mut out, (payload.len() + 8) as u32);
    out.extend_from_slice(name);
    out.extend_from_slice(payload);
    out
}

fn full_atom_payload(version: u8, flags: u32) -> Vec<u8> {
    vec![
        version,
        ((flags >> 16) & 0xFF) as u8,
        ((flags >> 8) & 0xFF) as u8,
        (flags & 0xFF) as u8,
    ]
}

fn write_descriptor(out: &mut Vec<u8>, tag: u8, payload: &[u8]) {
    out.push(tag);
    write_descriptor_len(out, payload.len());
    out.extend_from_slice(payload);
}

fn write_descriptor_len(out: &mut Vec<u8>, len: usize) {
    out.push(((len >> 21) as u8 & 0x7F) | 0x80);
    out.push(((len >> 14) as u8 & 0x7F) | 0x80);
    out.push(((len >> 7) as u8 & 0x7F) | 0x80);
    out.push((len as u8) & 0x7F);
}

fn write_matrix(out: &mut Vec<u8>) {
    write_u32(out, 0x0001_0000);
    write_u32(out, 0);
    write_u32(out, 0);
    write_u32(out, 0);
    write_u32(out, 0x0001_0000);
    write_u32(out, 0);
    write_u32(out, 0);
    write_u32(out, 0);
    write_u32(out, 0x4000_0000);
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn sample_rate_from_index(index: u8) -> Option<u32> {
    match index {
        0 => Some(96_000),
        1 => Some(88_200),
        2 => Some(64_000),
        3 => Some(48_000),
        4 => Some(44_100),
        5 => Some(32_000),
        6 => Some(24_000),
        7 => Some(22_050),
        8 => Some(16_000),
        9 => Some(12_000),
        10 => Some(11_025),
        11 => Some(8_000),
        12 => Some(7_350),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::adts_to_m4a;

    #[test]
    fn wraps_adts_frames_in_m4a_container() {
        let adts = two_silent_aac_frames();
        let m4a = adts_to_m4a(&adts, 128).expect("wrap adts");

        assert_eq!(&m4a[4..8], b"ftyp");
        assert!(contains_atom(&m4a, b"mdat"));
        assert!(contains_atom(&m4a, b"moov"));
        assert!(contains_atom(&m4a, b"mp4a"));
        assert!(contains_atom(&m4a, b"esds"));
    }

    #[test]
    fn rejects_non_adts_input() {
        let err = adts_to_m4a(b"not aac", 128).expect_err("reject invalid adts");
        assert!(err.to_string().contains("ADTS"));
    }

    fn two_silent_aac_frames() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&adts_frame(&[0x21, 0x10, 0x04]));
        out.extend_from_slice(&adts_frame(&[0x21, 0x10, 0x04]));
        out
    }

    fn adts_frame(payload: &[u8]) -> Vec<u8> {
        let frame_len = payload.len() + 7;
        let mut header = [0_u8; 7];
        header[0] = 0xFF;
        header[1] = 0xF1;
        header[2] = 0x50;
        header[3] = 0x80 | (((frame_len >> 11) & 0x03) as u8);
        header[4] = ((frame_len >> 3) & 0xFF) as u8;
        header[5] = (((frame_len & 0x07) << 5) as u8) | 0x1F;
        header[6] = 0xFC;

        let mut frame = header.to_vec();
        frame.extend_from_slice(payload);
        frame
    }

    fn contains_atom(data: &[u8], name: &[u8; 4]) -> bool {
        data.windows(4).any(|window| window == name)
    }
}
