# Sonic

Sonic is a lightweight Rust audio transcoder for embedding directly into your app via FFI (no HTTP service).

## What It Does

- Input: `MP3`, `WAV`, `FLAC`
- Output: `AAC` (ADTS), `M4A`, or `MP3`
- Presets:
  - `LOW` = `64 kbps`
  - `MEDIUM` = `128 kbps`
  - `HIGH` = `192 kbps`
  - `VERY_HIGH` = `320 kbps`
- Custom bitrate APIs for callers that need an exact target bitrate
- Options-based `SonicTranscodeOptions` and `SonicBuffer` APIs for easier host integration
- Probe API for format, sample rate, channels, duration, bit depth, and metadata/artwork presence
- Basic MP3 ID3 metadata/artwork preservation when writing MP3 or ADTS AAC
- Capability reporting for the current build
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
- `sonic_transcode_to_format_with_bitrate(...)`
- `sonic_transcode(...)` (recommended options-based buffer API)
- `sonic_transcode_file_to_format(...)`
- `sonic_transcode_file_to_format_with_bitrate(...)`
- `sonic_transcode_file(...)` (recommended options-based file API)
- `sonic_default_transcode_options()`
- `sonic_transcode_directory(...)` (batch directory API with configurable workers)
- `sonic_default_batch_options()`
- `sonic_probe_audio(...)`
- `sonic_get_capabilities()`
- `sonic_transcode_mp3_to_aac(...)` (compat helper)
- `sonic_transcode_mp3_file_to_aac_file(...)` (compat helper)
- `sonic_free_buffer(...)`
- `sonic_free_c_string(...)`
- `sonic_ffi_abi_version()`

## Notes

- AAC encoding requires building with `--features aac-fdk`.
- `M4A` output also requires `--features aac-fdk` because it wraps Sonic's AAC encoder output.
- Buffers returned by Sonic must be freed with `sonic_free_buffer`.
- `SonicBuffer` values returned by `sonic_transcode` must be freed with `sonic_free_output_buffer`.
- Error strings returned by Sonic must be freed with `sonic_free_c_string`.

## Batch Transcoding

Use the directory batch API when you want Sonic to manage parallel file transcoding:

```c
SonicBatchOptions batch = sonic_default_batch_options();
batch.transcode.output_format = SONIC_OUTPUT_M4A;
batch.transcode.preset = SONIC_PRESET_LOW;
batch.workers = 10; // 0 means Sonic chooses a default based on available parallelism.

SonicBatchResult result = {0};
char* error = NULL;
int32_t status = sonic_transcode_directory("Music Folder", "Output Folder", &batch, &result, &error);
```

## C Example

```c
SonicTranscodeOptions options = sonic_default_transcode_options();
options.output_format = SONIC_OUTPUT_M4A;
options.preset = SONIC_PRESET_HIGH;

SonicBuffer output = {0};
char* error = NULL;
int32_t status = sonic_transcode(input_bytes, input_len, &options, &output, &error);

if (status == SONIC_STATUS_OK) {
    // use output.ptr/output.len
    sonic_free_output_buffer(&output);
} else {
    // inspect error, then release it
    sonic_free_c_string(error);
}
```
