use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscodeError {
    #[error("empty request body")]
    EmptyBody,

    #[error("unsupported input format; expected MP3, WAV, or FLAC")]
    UnsupportedFormat,

    #[error("invalid preset '{0}'; expected one of: LOW, MEDIUM")]
    InvalidPreset(String),

    #[error("invalid output format '{0}'; expected one of: aac, mp3")]
    InvalidOutputFormat(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),
}
