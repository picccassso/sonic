use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Instant,
};

use sonic_transcoder::{audio::transcoder::Transcoder, errors::TranscodeError};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
struct BenchConfig {
    input_dir: PathBuf,
    workers: usize,
    bitrate_kbps: u32,
    strict: bool,
}

#[derive(Debug, Default, Clone)]
struct WorkerStats {
    attempted: u64,
    transcoded_ok: u64,
    decoded_only_ok: u64,
    failed: u64,
    read_failed: u64,
    input_bytes: u64,
    output_bytes: u64,
    transcode_nanos: u128,
    read_nanos: u128,
    error_samples: Vec<String>,
}

impl WorkerStats {
    fn merge(&mut self, other: WorkerStats) {
        self.attempted += other.attempted;
        self.transcoded_ok += other.transcoded_ok;
        self.decoded_only_ok += other.decoded_only_ok;
        self.failed += other.failed;
        self.read_failed += other.read_failed;
        self.input_bytes += other.input_bytes;
        self.output_bytes += other.output_bytes;
        self.transcode_nanos += other.transcode_nanos;
        self.read_nanos += other.read_nanos;

        let remaining = 10usize.saturating_sub(self.error_samples.len());
        self.error_samples
            .extend(other.error_samples.into_iter().take(remaining));
    }
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            print_usage();
            std::process::exit(2);
        }
    };

    let files = collect_mp3_files(&config.input_dir);
    if files.is_empty() {
        eprintln!("No .mp3 files found under: {}", config.input_dir.display());
        std::process::exit(1);
    }

    println!("Benchmark starting");
    println!("input_dir={}", config.input_dir.display());
    println!("files_found={}", files.len());
    println!("workers={}", config.workers);
    println!("bitrate_kbps={}", config.bitrate_kbps);
    println!("strict={}", config.strict);

    let files = Arc::new(files);
    let next_index = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();

    let mut handles = Vec::with_capacity(config.workers);
    for _ in 0..config.workers {
        let files = Arc::clone(&files);
        let next_index = Arc::clone(&next_index);
        let config = config.clone();

        handles.push(thread::spawn(move || {
            let mut stats = WorkerStats::default();
            let transcoder = Transcoder::new(config.bitrate_kbps);

            loop {
                let idx = next_index.fetch_add(1, Ordering::Relaxed);
                if idx >= files.len() {
                    break;
                }

                stats.attempted += 1;
                let path = &files[idx];

                let read_start = Instant::now();
                let input = match fs::read(path) {
                    Ok(v) => v,
                    Err(err) => {
                        stats.failed += 1;
                        stats.read_failed += 1;
                        if stats.error_samples.len() < 3 {
                            stats.error_samples.push(format!(
                                "read failed {}: {}",
                                path.display(),
                                err
                            ));
                        }
                        continue;
                    }
                };
                stats.read_nanos += read_start.elapsed().as_nanos();
                stats.input_bytes += input.len() as u64;

                let transcode_start = Instant::now();
                match transcoder.transcode(&input) {
                    Ok(output) => {
                        stats.transcoded_ok += 1;
                        stats.output_bytes += output.len() as u64;
                    }
                    Err(TranscodeError::NotImplemented(_)) if !config.strict => {
                        // Current scaffold has AAC encoding stubbed. Count decode success.
                        stats.decoded_only_ok += 1;
                    }
                    Err(err) => {
                        stats.failed += 1;
                        if stats.error_samples.len() < 3 {
                            stats.error_samples
                                .push(format!("transcode failed {}: {}", path.display(), err));
                        }
                    }
                }
                stats.transcode_nanos += transcode_start.elapsed().as_nanos();
            }

            stats
        }));
    }

    let mut total = WorkerStats::default();
    for handle in handles {
        match handle.join() {
            Ok(stats) => total.merge(stats),
            Err(_) => {
                eprintln!("A worker thread panicked");
                std::process::exit(1);
            }
        }
    }

    let wall = started.elapsed();
    let wall_secs = wall.as_secs_f64();
    let attempted = total.attempted.max(1);

    println!("\nResults");
    println!("attempted={}", total.attempted);
    println!("transcoded_ok={}", total.transcoded_ok);
    println!("decoded_only_ok={}", total.decoded_only_ok);
    println!("failed={}", total.failed);
    println!("read_failed={}", total.read_failed);
    println!("input_mib={:.2}", total.input_bytes as f64 / 1_048_576.0);
    println!("output_mib={:.2}", total.output_bytes as f64 / 1_048_576.0);
    println!("wall_time_s={:.3}", wall_secs);
    println!(
        "throughput_files_per_s={:.2}",
        total.attempted as f64 / wall_secs.max(f64::EPSILON)
    );
    println!(
        "throughput_input_mib_per_s={:.2}",
        (total.input_bytes as f64 / 1_048_576.0) / wall_secs.max(f64::EPSILON)
    );
    println!(
        "avg_read_ms_per_file={:.2}",
        (total.read_nanos as f64 / 1_000_000.0) / attempted as f64
    );
    println!(
        "avg_transcode_ms_per_file={:.2}",
        (total.transcode_nanos as f64 / 1_000_000.0) / attempted as f64
    );

    if !total.error_samples.is_empty() {
        println!("\nSample errors:");
        for err in &total.error_samples {
            println!("- {err}");
        }
    }

    if total.transcoded_ok == 0 && total.decoded_only_ok > 0 {
        println!(
            "\nNote: AAC encode is still stubbed, so this benchmark currently measures MP3 decode + pipeline overhead."
        );
    }
}

fn parse_args() -> Result<BenchConfig, String> {
    let mut args = std::env::args().skip(1);

    let mut input_dir: Option<PathBuf> = None;
    let mut workers: usize = num_cpus::get();
    let mut bitrate_kbps: u32 = 128;
    let mut strict = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workers" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --workers".to_string())?;
                workers = value
                    .parse::<usize>()
                    .map_err(|_| "--workers must be a positive integer".to_string())?;
                if workers == 0 {
                    return Err("--workers must be > 0".to_string());
                }
            }
            "--bitrate" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --bitrate".to_string())?;
                bitrate_kbps = value
                    .parse::<u32>()
                    .map_err(|_| "--bitrate must be a positive integer".to_string())?;
                if bitrate_kbps == 0 {
                    return Err("--bitrate must be > 0".to_string());
                }
            }
            "--strict" => {
                strict = true;
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {
                if input_dir.is_none() {
                    input_dir = Some(PathBuf::from(arg));
                } else {
                    return Err(format!("unexpected argument: {arg}"));
                }
            }
        }
    }

    let input_dir = input_dir.ok_or_else(|| "missing input directory argument".to_string())?;
    if !input_dir.exists() {
        return Err(format!("input directory does not exist: {}", input_dir.display()));
    }
    if !input_dir.is_dir() {
        return Err(format!("input path is not a directory: {}", input_dir.display()));
    }

    Ok(BenchConfig {
        input_dir,
        workers,
        bitrate_kbps,
        strict,
    })
}

fn collect_mp3_files(root: &Path) -> Vec<PathBuf> {
    let mut files = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            let is_mp3 = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("mp3"))
                .unwrap_or(false);
            if is_mp3 {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    files.sort();
    files
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run --release --bin bench -- <input_dir> [--workers N] [--bitrate K] [--strict]"
    );
}
