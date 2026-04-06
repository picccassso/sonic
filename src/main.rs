use actix_web::{web, App, HttpServer};
use sonic_transcoder::{
    audio::transcoder::Transcoder, config::ServiceConfig, http::handlers::configure_routes,
};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let config = ServiceConfig::from_env();
    let transcoder = web::Data::new(Transcoder::new(config.aac_bitrate_kbps));

    log::info!(
        "starting sonic-transcoder on {} (workers={}, aac_bitrate={}k)",
        config.bind_addr,
        config.workers,
        config.aac_bitrate_kbps
    );

    HttpServer::new(move || {
        App::new()
            .app_data(transcoder.clone())
            .configure(configure_routes)
    })
    .workers(config.workers)
    .bind(&config.bind_addr)?
    .run()
    .await
}
