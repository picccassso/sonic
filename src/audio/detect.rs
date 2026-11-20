use std::{fs::File, io::Read, path::Path};

use crate::errors::TranscodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Mp3,
    Wav,
    Flac,
}

pub fn detect_format_from_path(path: &Path) -> Result<InputFormat, TranscodeError> {
    let mut file = File::open(path).map_err(|err| {
        TranscodeError::Decode(format!(
            "failed to open input file '{}': {err}",
            path.display()
        ))
    })?;
    let mut header = [0u8; 12];
    let n = file.read(&mut header).map_err(|err| {
        TranscodeError::Decode(format!(
            "failed to read input header '{}': {err}",
            path.display()
        ))
    })?;
    detect_format(&header[..n])
}

pub fn detect_format(input: &[u8]) -> Result<InputFormat, TranscodeError> {
    if input.is_empty() {
        return Err(TranscodeError::EmptyBody);
    }

    if looks_like_wav(input) {
        return Ok(InputFormat::Wav);
    }

    if input.starts_with(b"fLaC") {
        return Ok(InputFormat::Flac);
    }

    // MP3 can begin with ID3 tag or frame sync (0xFFE).
    if input.starts_with(b"ID3") || looks_like_mp3_frame_sync(input) {
        return Ok(InputFormat::Mp3);
    }

    Err(TranscodeError::UnsupportedFormat)
}

fn looks_like_mp3_frame_sync(input: &[u8]) -> bool {
    input
        .get(0..2)
        .map(|h| h[0] == 0xFF && (h[1] & 0xE0) == 0xE0)
        .unwrap_or(false)
}

fn looks_like_wav(input: &[u8]) -> bool {
    input.len() >= 12 && &input[0..4] == b"RIFF" && &input[8..12] == b"WAVE"
}
