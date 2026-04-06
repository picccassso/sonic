use crate::errors::TranscodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Mp3,
}

pub fn detect_format(input: &[u8]) -> Result<InputFormat, TranscodeError> {
    if input.is_empty() {
        return Err(TranscodeError::EmptyBody);
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
