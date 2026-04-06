use actix_web::{post, web, HttpResponse};
use serde::Deserialize;

use crate::{
    audio::{preset::QualityPreset, transcoder::Transcoder},
    errors::TranscodeError,
};

#[derive(Debug, Deserialize)]
struct TranscodeQuery {
    preset: Option<String>,
}

#[post("/transcode")]
async fn transcode_endpoint(
    query: web::Query<TranscodeQuery>,
    body: web::Bytes,
    transcoder: web::Data<Transcoder>,
) -> Result<HttpResponse, TranscodeError> {
    let bitrate_kbps = match query.preset.as_deref() {
        Some(raw) => {
            let preset = QualityPreset::from_trigger(raw)
                .ok_or_else(|| TranscodeError::InvalidPreset(raw.to_string()))?;
            preset.bitrate_kbps()
        }
        None => transcoder.default_bitrate_kbps(),
    };

    let aac = transcoder.transcode_with_bitrate(&body, bitrate_kbps)?;

    Ok(HttpResponse::Ok()
        .insert_header(("X-AAC-Bitrate-Kbps", bitrate_kbps.to_string()))
        .content_type("audio/aac")
        .body(aac))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(transcode_endpoint);
}
