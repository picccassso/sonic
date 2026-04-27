/// FFI status: success.
pub const SONIC_STATUS_OK: i32 = 0;
/// FFI status: one or more arguments were null/invalid.
pub const SONIC_STATUS_INVALID_ARGS: i32 = 1;
/// FFI status: unsupported input format.
pub const SONIC_STATUS_UNSUPPORTED_FORMAT: i32 = 2;
/// FFI status: decode failed.
pub const SONIC_STATUS_DECODE_ERROR: i32 = 3;
/// FFI status: encode failed.
pub const SONIC_STATUS_ENCODE_ERROR: i32 = 4;
/// FFI status: operation not implemented in current build.
pub const SONIC_STATUS_NOT_IMPLEMENTED: i32 = 5;
/// FFI status: quality preset value is invalid.
pub const SONIC_STATUS_INVALID_PRESET: i32 = 6;
/// FFI status: internal failure.
pub const SONIC_STATUS_INTERNAL_ERROR: i32 = 7;
/// FFI status: output format value is invalid.
pub const SONIC_STATUS_INVALID_OUTPUT_FORMAT: i32 = 8;

/// Quality presets accepted by preset-based transcode APIs.
pub const SONIC_PRESET_LOW: u32 = 0;
pub const SONIC_PRESET_MEDIUM: u32 = 1;
pub const SONIC_PRESET_HIGH: u32 = 2;
pub const SONIC_PRESET_VERY_HIGH: u32 = 3;

pub const SONIC_OUTPUT_AAC: u32 = 0;
pub const SONIC_OUTPUT_MP3: u32 = 1;
pub const SONIC_OUTPUT_M4A: u32 = 2;

pub const SONIC_INPUT_MP3: u32 = 0;
pub const SONIC_INPUT_WAV: u32 = 1;
pub const SONIC_INPUT_FLAC: u32 = 2;

pub const SONIC_CAP_INPUT_MP3: u32 = 1 << 0;
pub const SONIC_CAP_INPUT_WAV: u32 = 1 << 1;
pub const SONIC_CAP_INPUT_FLAC: u32 = 1 << 2;
pub const SONIC_CAP_OUTPUT_AAC: u32 = 1 << 8;
pub const SONIC_CAP_OUTPUT_MP3: u32 = 1 << 9;
pub const SONIC_CAP_OUTPUT_M4A: u32 = 1 << 10;
pub const SONIC_CAP_AAC_FDK: u32 = 1 << 16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl SonicBuffer {
    pub fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicTranscodeOptions {
    pub output_format: u32,
    pub preset: u32,
    pub bitrate_kbps: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicBatchOptions {
    pub transcode: SonicTranscodeOptions,
    pub workers: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicBatchResult {
    pub files_total: u64,
    pub files_completed: u64,
    pub files_failed: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub workers_used: u32,
}

impl SonicBatchResult {
    pub fn empty() -> Self {
        Self {
            files_total: 0,
            files_completed: 0,
            files_failed: 0,
            input_bytes: 0,
            output_bytes: 0,
            workers_used: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicAudioInfo {
    pub input_format: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: u32,
    pub duration_ms: u64,
    pub total_samples_per_channel: u64,
    pub has_metadata: u32,
    pub has_artwork: u32,
}

impl SonicAudioInfo {
    pub fn empty() -> Self {
        Self {
            input_format: 0,
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            duration_ms: 0,
            total_samples_per_channel: 0,
            has_metadata: 0,
            has_artwork: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicCapabilities {
    pub abi_version: u32,
    pub input_formats: u32,
    pub output_formats: u32,
    pub features: u32,
    pub preset_count: u32,
}
