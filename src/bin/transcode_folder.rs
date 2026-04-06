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

use sonic_transcoder::audio::transcoder::Transcoder;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
struct Config {
    input_dir: PathBuf,
    output_dir: PathBuf,
    workers: usize,
    bitrate_kbps: u32,
}

#[derive(Debug, Clone)]
struct Job {
    input: PathBuf,
    output: PathBuf,
}

#[derive(Debug, Default, Clone)]
struct WorkerStats {
    processed: u64,
    succeeded: u64,
    failed: u64,
    input_bytes: u64,
    output_bytes: u64,
    transcode_nanos: u128,
    write_nanos: u128,
    errors: Vec<String>,
}

impl WorkerStats {
    fn merge(&mut self, other: WorkerStats) {
        self.processed += other.processed;
        self.succeeded += other.succeeded;
        self.failed += other.failed;
        self.input_bytes += other.input_bytes;
        self.output_bytes += other.output_bytes;
        self.transcode_nanos += other.transcode_nanos;
        self.write_nanos += other.write_nanos;

        let remaining = 12usize.saturating_sub(self.errors.len());
        self.errors.extend(other.errors.into_iter().take(remaining));
    }
}

fn main() {
    let config = match parse_args() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            print_usage();
            std::process::exit(2);
        }
    };

    let jobs = collect_jobs(&config.input_dir, &config.output_dir);
    if jobs.is_empty() {
        eprintln!("No .mp3 files found under {}", config.input_dir.display());
        std::process::exit(1);
    }

    if let Err(err) = fs::create_dir_all(&config.output_dir) {
        eprintln!(
            "Failed to create output directory {}: {}",
            config.output_dir.display(),
            err
        );
        std::process::exit(1);
    }

    println!("Folder transcode starting");
    println!("input_dir={}", config.input_dir.display());
    println!("output_dir={}", config.output_dir.display());
    println!("workers={}", config.workers);
    println!("bitrate_kbps={}", config.bitrate_kbps);
    println!("files={}", jobs.len());

    let start = Instant::now();
    let jobs = Arc::new(jobs);
    let next_index = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(config.workers);
    for _ in 0..config.workers {
        let jobs = Arc::clone(&jobs);
        let next_index = Arc::clone(&next_index);
        let config = config.clone();

        handles.push(thread::spawn(move || {
            let transcoder = Transcoder::new(config.bitrate_kbps);
            let mut stats = WorkerStats::default();

            loop {
                let idx = next_index.fetch_add(1, Ordering::Relaxed);
                if idx >= jobs.len() {
                    break;
                }

                let job = &jobs[idx];
                stats.processed += 1;

                let input = match fs::read(&job.input) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        stats.failed += 1;
                        if stats.errors.len() < 3 {
                            stats.errors.push(format!(
                                "read failed {}: {}",
                                job.input.display(),
                                err
                            ));
                        }
                        continue;
                    }
                };
                stats.input_bytes += input.len() as u64;

                let transcode_started = Instant::now();
                let output = match transcoder.transcode(&input) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        stats.failed += 1;
                        if stats.errors.len() < 3 {
                            stats.errors.push(format!(
                                "transcode failed {}: {}",
                                job.input.display(),
                                err
                            ));
                        }
                        continue;
                    }
                };
                stats.transcode_nanos += transcode_started.elapsed().as_nanos();
                stats.output_bytes += output.len() as u64;

                if let Some(parent) = job.output.parent() {
                    if let Err(err) = fs::create_dir_all(parent) {
                        stats.failed += 1;
                        if stats.errors.len() < 3 {
                            stats.errors.push(format!(
                                "mkdir failed {}: {}",
                                parent.display(),
                                err
                            ));
                        }
                        continue;
                    }
                }

                let write_started = Instant::now();
                if let Err(err) = fs::write(&job.output, output) {
                    stats.failed += 1;
                    if stats.errors.len() < 3 {
                        stats.errors.push(format!(
                            "write failed {}: {}",
                            job.output.display(),
                            err
                        ));
                    }
                    continue;
                }
                stats.write_nanos += write_started.elapsed().as_nanos();

                stats.succeeded += 1;
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

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    let processed = total.processed.max(1);

    println!("\nResults");
    println!("processed={}", total.processed);
    println!("succeeded={}", total.succeeded);
    println!("failed={}", total.failed);
    println!("input_mib={:.2}", total.input_bytes as f64 / 1_048_576.0);
    println!("output_mib={:.2}", total.output_bytes as f64 / 1_048_576.0);
    println!("elapsed_s={:.3}", elapsed_secs);
    println!(
        "files_per_second={:.2}",
        total.processed as f64 / elapsed_secs.max(f64::EPSILON)
    );
    println!(
        "input_mib_per_second={:.2}",
        (total.input_bytes as f64 / 1_048_576.0) / elapsed_secs.max(f64::EPSILON)
    );
    println!(
        "avg_transcode_ms={:.2}",
        (total.transcode_nanos as f64 / 1_000_000.0) / processed as f64
    );
    println!(
        "avg_write_ms={:.2}",
        (total.write_nanos as f64 / 1_000_000.0) / processed as f64
    );

    if !total.errors.is_empty() {
        println!("\nSample errors:");
        for err in &total.errors {
            println!("- {err}");
        }
    }

    if total.failed > 0 {
        std::process::exit(1);
    }
}

fn parse_args() -> Result<Config, String> {
    let mut args = std::env::args().skip(1);

    let input_dir = args
        .next()
        .ok_or_else(|| "missing input directory argument".to_string())?;
    let output_dir = args
        .next()
        .ok_or_else(|| "missing output directory argument".to_string())?;

    let mut workers = num_cpus::get();
    let mut bitrate_kbps = 128u32;

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
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            _ => return Err(format!("unexpected argument: {arg}")),
        }
    }

    let input_dir = PathBuf::from(input_dir);
    let output_dir = PathBuf::from(output_dir);

    if !input_dir.exists() {
        return Err(format!("input directory does not exist: {}", input_dir.display()));
    }
    if !input_dir.is_dir() {
        return Err(format!("input path is not a directory: {}", input_dir.display()));
    }

    Ok(Config {
        input_dir,
        output_dir,
        workers,
        bitrate_kbps,
    })
}

fn collect_jobs(input_root: &Path, output_root: &Path) -> Vec<Job> {
    let mut jobs = Vec::new();

    for entry in WalkDir::new(input_root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let in_path = entry.into_path();
        let is_mp3 = in_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("mp3"))
            .unwrap_or(false);

        if !is_mp3 {
            continue;
        }

        let rel = match in_path.strip_prefix(input_root) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let mut out_path = output_root.join(rel);
        out_path.set_extension("aac");

        jobs.push(Job {
            input: in_path,
            output: out_path,
        });
    }

    jobs.sort_by(|a, b| a.input.cmp(&b.input));
    jobs
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run --release --features aac-fdk --bin transcode_folder -- <input_dir> <output_dir> [--workers N] [--bitrate K]"
    );
}
