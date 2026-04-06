use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscodeError {
    #[error("empty request body")]
    EmptyBody,

    #[error("unsupported input format; expected MP3")]
    UnsupportedFormat,

    #[error("invalid preset '{0}'; expected one of: LOW, MEDIUM")]
    InvalidPreset(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

impl ResponseError for TranscodeError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::EmptyBody => StatusCode::BAD_REQUEST,
            Self::InvalidPreset(_) => StatusCode::BAD_REQUEST,
            Self::UnsupportedFormat => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Decode(_) | Self::Encode(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).body(self.to_string())
    }
}
