# sonic-transcoder

Lightweight Rust audio transcoding service for MP3 -> AAC, built with Actix-web.

## Features

- `POST /transcode` accepts raw MP3 bytes and returns AAC bytes
- AAC encoding via `libfdk-aac` (feature: `aac-fdk`)
- Presets via query param:
  - `preset=LOW` -> 64 kbps
  - `preset=MEDIUM` -> 128 kbps
- Optional artwork carry-over from MP3 ID3 cover art

## Run

```bash
cargo run --features aac-fdk
```

Server defaults:

- bind: `0.0.0.0:8080`
- workers: CPU count
- bitrate: `128` kbps (used when no preset is provided)

## Transcode API

```bash
curl -X POST "http://127.0.0.1:8080/transcode?preset=MEDIUM" \
  --data-binary @input.mp3 \
  -o output.aac
```

## Folder Benchmark

Rust folder transcode benchmark:

```bash
cargo run --release --features aac-fdk --bin transcode_folder -- "Music Folder" "Music Folder Test" --workers 10 --bitrate 128
```

FFmpeg comparison benchmark:

```bash
./scripts/bench_ffmpeg_folder.sh "Music Folder" "Music Folder Test ffmpeg" 10 128
```
