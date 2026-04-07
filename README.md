# Sonic

Sonic is a lightweight Rust audio transcoder for embedding directly into your app via FFI (no HTTP service).

## What It Does

- Input: `MP3`, `WAV`, `FLAC`
- Output: `AAC` (ADTS) or `MP3`
- Presets:
  - `LOW` = `64 kbps`
  - `MEDIUM` = `128 kbps`
- FFI API for desktop/headless builds on macOS, Linux, and Windows

## Basic Setup

1. Build the shared library:

```bash
cargo build --release --features aac-fdk --lib
```

2. Use the generated library:
- macOS: `target/release/libsonic_transcoder.dylib`
- Linux: `target/release/libsonic_transcoder.so`
- Windows: `target/release/sonic_transcoder.dll`

3. Include the C header in your host app:
- `include/sonic_ffi.h`

## FFI Entry Points

- `sonic_transcode_to_format(...)` (recommended)
- `sonic_transcode_mp3_to_aac(...)` (compat helper)
- `sonic_free_buffer(...)`
- `sonic_free_c_string(...)`
- `sonic_ffi_abi_version()`

## Notes

- AAC encoding requires building with `--features aac-fdk`.
- Buffers returned by Sonic must be freed with `sonic_free_buffer`.
- Error strings returned by Sonic must be freed with `sonic_free_c_string`.
