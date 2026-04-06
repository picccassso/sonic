use std::env;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub bind_addr: String,
    pub aac_bitrate_kbps: u32,
    pub workers: usize,
}

impl ServiceConfig {
    pub fn from_env() -> Self {
        let bind_addr = env::var("SONIC_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let aac_bitrate_kbps = env::var("SONIC_AAC_BITRATE_KBPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(128);

        let workers = env::var("SONIC_HTTP_WORKERS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or_else(num_cpus::get);

        Self {
            bind_addr,
            aac_bitrate_kbps,
            workers,
        }
    }
}
