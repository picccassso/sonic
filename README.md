# sonic-transcoder

Rust MP3 -> AAC transcoder with two integration modes:

- HTTP service (`POST /transcode`)
- Embedded native library (C ABI via `cdylib`)

## Current scope

- Input: **MP3 only**
- Output: **AAC (ADTS)** bytes
- Presets:
  - `LOW` -> `64 kbps`
  - `MEDIUM` -> `128 kbps`
- Best-effort cover-art carry-over from MP3 ID3 artwork
- If AAC encoder feature is not enabled, encode returns `not implemented`

## Build

Enable AAC encoding with `aac-fdk`:

```bash
cargo build --features aac-fdk
```

## HTTP mode

Run server:

```bash
cargo run --features aac-fdk
```

Defaults:

- `SONIC_BIND_ADDR=0.0.0.0:8080`
- `SONIC_HTTP_WORKERS=<cpu_count>`
- `SONIC_AAC_BITRATE_KBPS=128` (used when `preset` is omitted)

Request example:

```bash
curl -X POST "http://127.0.0.1:8080/transcode?preset=MEDIUM" \
  --data-binary @input.mp3 \
  -o output.aac
```

Accepted `preset` values: `LOW`, `MEDIUM`.

## Embedded mode (FFI, no HTTP)

Build shared library:

```bash
cargo build --release --features aac-fdk --lib
```

Artifacts:

- macOS: `target/release/libsonic_transcoder.dylib`
- Linux: `target/release/libsonic_transcoder.so`
- Windows: `target/release/sonic_transcoder.dll`

C header:

- `include/sonic_ffi.h`

Exported functions:

- `sonic_transcode_mp3_to_aac(...)`
- `sonic_free_buffer(...)`
- `sonic_free_c_string(...)`
- `sonic_ffi_abi_version()`

FFI presets:

- `SONIC_PRESET_LOW` (`0`) -> 64 kbps
- `SONIC_PRESET_MEDIUM` (`1`) -> 128 kbps

Memory ownership:

- Output AAC buffer returned by `sonic_transcode_mp3_to_aac` must be freed with `sonic_free_buffer`.
- Error string (if returned) must be freed with `sonic_free_c_string`.

## Benchmark tools

Rust folder transcode benchmark:

```bash
cargo run --release --features aac-fdk --bin transcode_folder -- "Music Folder" "Music Folder Test" --workers 10 --bitrate 128
```

FFmpeg comparison benchmark:

```bash
./scripts/bench_ffmpeg_folder.sh "Music Folder" "Music Folder Test ffmpeg" 10 128
```
