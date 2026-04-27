use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use crate::audio::{output::OutputFormat, preset::QualityPreset, transcoder::Transcoder};

#[derive(Debug, Clone, Copy)]
pub struct BatchTranscodeOptions {
    pub output_format: OutputFormat,
    pub preset: QualityPreset,
    pub bitrate_kbps: Option<u32>,
    pub workers: usize,
}

#[derive(Debug, Clone, Default)]
pub struct BatchTranscodeSummary {
    pub files_total: u64,
    pub files_completed: u64,
    pub files_failed: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub workers_used: usize,
}

#[derive(Debug, Clone)]
struct BatchJob {
    input_path: PathBuf,
    output_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct WorkerSummary {
    files_completed: u64,
    files_failed: u64,
    input_bytes: u64,
    output_bytes: u64,
    first_error: Option<String>,
}

pub fn transcode_directory(
    input_dir: &Path,
    output_dir: &Path,
    options: BatchTranscodeOptions,
) -> Result<BatchTranscodeSummary, String> {
    if !input_dir.is_dir() {
        return Err(format!("input_dir is not a directory: {}", input_dir.display()));
    }

    fs::create_dir_all(output_dir)
        .map_err(|err| format!("failed to create output_dir '{}': {err}", output_dir.display()))?;

    let jobs = collect_jobs(input_dir, output_dir, options.output_format)?;
    let files_total = jobs.len() as u64;
    if jobs.is_empty() {
        return Ok(BatchTranscodeSummary {
            workers_used: 0,
            ..BatchTranscodeSummary::default()
        });
    }

    let workers = resolve_workers(options.workers).min(jobs.len());
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let options = options;
        handles.push(thread::spawn(move || worker_loop(queue, options)));
    }

    let mut summary = BatchTranscodeSummary {
        files_total,
        workers_used: workers,
        ..BatchTranscodeSummary::default()
    };
    for handle in handles {
        let worker = handle
            .join()
            .map_err(|_| "batch worker panicked".to_string())?;
        summary.files_completed += worker.files_completed;
        summary.files_failed += worker.files_failed;
        summary.input_bytes += worker.input_bytes;
        summary.output_bytes += worker.output_bytes;
    }

    Ok(summary)
}

fn worker_loop(
    queue: Arc<Mutex<VecDeque<BatchJob>>>,
    options: BatchTranscodeOptions,
) -> WorkerSummary {
    let mut summary = WorkerSummary::default();
    let transcoder = Transcoder::new(
        options
            .bitrate_kbps
            .unwrap_or_else(|| options.preset.bitrate_kbps()),
    );

    loop {
        let job = {
            let mut queue = queue.lock().expect("batch queue mutex poisoned");
            queue.pop_front()
        };

        let Some(job) = job else {
            break;
        };

        match transcode_job(&transcoder, &job, options) {
            Ok((input_bytes, output_bytes)) => {
                summary.files_completed += 1;
                summary.input_bytes += input_bytes;
                summary.output_bytes += output_bytes;
            }
            Err(err) => {
                summary.files_failed += 1;
                if summary.first_error.is_none() {
                    summary.first_error = Some(err);
                }
            }
        }
    }

    summary
}

fn transcode_job(
    transcoder: &Transcoder,
    job: &BatchJob,
    options: BatchTranscodeOptions,
) -> Result<(u64, u64), String> {
    let input = fs::read(&job.input_path)
        .map_err(|err| format!("failed to read '{}': {err}", job.input_path.display()))?;
    let input_bytes = input.len() as u64;

    if let Some(parent) = job.output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create '{}': {err}", parent.display()))?;
    }

    let output = if let Some(bitrate_kbps) = options.bitrate_kbps {
        transcoder
            .transcode_with_bitrate_and_format(&input, bitrate_kbps, options.output_format)
    } else {
        transcoder.transcode_with_preset_and_format(&input, options.preset, options.output_format)
    }
    .map_err(|err| format!("failed to transcode '{}': {err}", job.input_path.display()))?;

    let output_bytes = output.len() as u64;
    fs::write(&job.output_path, output)
        .map_err(|err| format!("failed to write '{}': {err}", job.output_path.display()))?;

    Ok((input_bytes, output_bytes))
}

fn collect_jobs(
    input_dir: &Path,
    output_dir: &Path,
    output_format: OutputFormat,
) -> Result<Vec<BatchJob>, String> {
    let mut jobs = Vec::new();
    collect_jobs_inner(input_dir, input_dir, output_dir, output_format, &mut jobs)?;
    jobs.sort_by(|a, b| a.input_path.cmp(&b.input_path));
    Ok(jobs)
}

fn collect_jobs_inner(
    root: &Path,
    current: &Path,
    output_dir: &Path,
    output_format: OutputFormat,
    jobs: &mut Vec<BatchJob>,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|err| format!("failed to read directory '{}': {err}", current.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read file type for '{}': {err}", path.display()))?;

        if file_type.is_dir() {
            collect_jobs_inner(root, &path, output_dir, output_format, jobs)?;
            continue;
        }

        if !file_type.is_file() || !is_supported_audio_path(&path) {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .map_err(|err| format!("failed to create relative path for '{}': {err}", path.display()))?;
        let output_path = output_dir
            .join(rel)
            .with_extension(output_format.file_extension());

        jobs.push(BatchJob {
            input_path: path,
            output_path,
        });
    }

    Ok(())
}

fn is_supported_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mp3" | "wav" | "flac"))
        .unwrap_or(false)
}

fn resolve_workers(workers: usize) -> usize {
    if workers > 0 {
        return workers;
    }

    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use hound::{SampleFormat, WavSpec, WavWriter};

    use super::{transcode_directory, BatchTranscodeOptions};
    use crate::audio::{output::OutputFormat, preset::QualityPreset};

    #[test]
    fn batch_transcodes_supported_files_with_workers() {
        let root = temp_dir("sonic-batch-test");
        let input = root.join("input");
        let output = root.join("output");
        fs::create_dir_all(input.join("album")).expect("create input dirs");

        fs::write(input.join("album").join("one.wav"), tiny_wav()).expect("write first wav");
        fs::write(input.join("two.wav"), tiny_wav()).expect("write second wav");
        fs::write(input.join("skip.txt"), b"skip").expect("write skipped file");

        let summary = transcode_directory(
            &input,
            &output,
            BatchTranscodeOptions {
                output_format: OutputFormat::Mp3,
                preset: QualityPreset::Low,
                bitrate_kbps: None,
                workers: 2,
            },
        )
        .expect("batch transcode");

        assert_eq!(summary.files_total, 2);
        assert_eq!(summary.files_completed, 2);
        assert_eq!(summary.files_failed, 0);
        assert_eq!(summary.workers_used, 2);
        assert!(output.join("album").join("one.mp3").exists());
        assert!(output.join("two.mp3").exists());
        assert!(!output.join("skip.mp3").exists());

        let _ = fs::remove_dir_all(root);
    }

    fn tiny_wav() -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let spec = WavSpec {
                channels: 1,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::new(&mut cursor, spec).expect("create wav writer");
            for _ in 0..2048 {
                writer.write_sample::<i16>(0).expect("write sample");
            }
            writer.finalize().expect("finalize wav");
        }
        cursor.into_inner()
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{id}"))
    }
}
