use std::{
    ffi::{c_char, CStr, CString},
    fs,
};

use crate::{
    audio::{
        detect::InputFormat,
        output::OutputFormat,
        preset::QualityPreset,
        probe::{self, AudioInfo},
        transcoder::Transcoder,
    },
    errors::TranscodeError,
};

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
/// FFI status: output format value is invalid.
pub const SONIC_STATUS_INVALID_OUTPUT_FORMAT: i32 = 8;
/// FFI status: internal failure.
pub const SONIC_STATUS_INTERNAL_ERROR: i32 = 7;

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

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonicCapabilities {
    pub abi_version: u32,
    pub input_formats: u32,
    pub output_formats: u32,
    pub features: u32,
    pub preset_count: u32,
}

/// Transcode MP3 bytes to AAC bytes with a quality preset.
///
/// Ownership model:
/// - On success, output bytes are allocated by Sonic and returned via out params.
/// - Caller must release output via `sonic_free_buffer`.
/// - On error, `out_error` may contain an allocated C string; release via
///   `sonic_free_c_string`.
///
/// Returns one of SONIC_STATUS_* constants.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_mp3_to_aac(
    input_ptr: *const u8,
    input_len: usize,
    preset: u32,
    out_data_ptr: *mut *mut u8,
    out_data_len: *mut usize,
    out_data_cap: *mut usize,
    out_error: *mut *mut c_char,
) -> i32 {
    sonic_transcode_to_format(
        input_ptr,
        input_len,
        preset,
        SONIC_OUTPUT_AAC,
        out_data_ptr,
        out_data_len,
        out_data_cap,
        out_error,
    )
}

/// Transcode MP3/WAV/FLAC bytes to AAC, M4A, or MP3 bytes.
///
/// Presets:
/// - SONIC_PRESET_LOW
/// - SONIC_PRESET_MEDIUM
/// - SONIC_PRESET_HIGH
/// - SONIC_PRESET_VERY_HIGH
///
/// Output formats:
/// - SONIC_OUTPUT_AAC
/// - SONIC_OUTPUT_M4A
/// - SONIC_OUTPUT_MP3
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_to_format(
    input_ptr: *const u8,
    input_len: usize,
    preset: u32,
    output_format: u32,
    out_data_ptr: *mut *mut u8,
    out_data_len: *mut usize,
    out_data_cap: *mut usize,
    out_error: *mut *mut c_char,
) -> i32 {
    sonic_transcode_to_format_inner(
        input_ptr,
        input_len,
        preset,
        output_format,
        out_data_ptr,
        out_data_len,
        out_data_cap,
        out_error,
    )
}

/// Transcode MP3/WAV/FLAC bytes to AAC, M4A, or MP3 bytes with an explicit bitrate.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_to_format_with_bitrate(
    input_ptr: *const u8,
    input_len: usize,
    bitrate_kbps: u32,
    output_format: u32,
    out_data_ptr: *mut *mut u8,
    out_data_len: *mut usize,
    out_data_cap: *mut usize,
    out_error: *mut *mut c_char,
) -> i32 {
    sonic_transcode_to_format_with_bitrate_inner(
        input_ptr,
        input_len,
        bitrate_kbps,
        output_format,
        out_data_ptr,
        out_data_len,
        out_data_cap,
        out_error,
    )
}

unsafe fn sonic_transcode_to_format_inner(
    input_ptr: *const u8,
    input_len: usize,
    preset: u32,
    output_format: u32,
    out_data_ptr: *mut *mut u8,
    out_data_len: *mut usize,
    out_data_cap: *mut usize,
    out_error: *mut *mut c_char,
) -> i32 {
    if out_data_ptr.is_null() || out_data_len.is_null() || out_data_cap.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    // Initialize outputs to a known state.
    *out_data_ptr = std::ptr::null_mut();
    *out_data_len = 0;
    *out_data_cap = 0;

    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    if input_ptr.is_null() && input_len > 0 {
        write_error(
            out_error,
            "input_ptr is null while input_len is non-zero".to_string(),
        );
        return SONIC_STATUS_INVALID_ARGS;
    }

    let input = std::slice::from_raw_parts(input_ptr, input_len);

    let quality = match parse_preset(preset) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid preset value {preset}; expected SONIC_PRESET_LOW ({SONIC_PRESET_LOW}), SONIC_PRESET_MEDIUM ({SONIC_PRESET_MEDIUM}), SONIC_PRESET_HIGH ({SONIC_PRESET_HIGH}), or SONIC_PRESET_VERY_HIGH ({SONIC_PRESET_VERY_HIGH})"
                ),
            );
            return SONIC_STATUS_INVALID_PRESET;
        }
    };

    let output_format = match parse_output_format(output_format) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}), SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3}), or SONIC_OUTPUT_M4A ({SONIC_OUTPUT_M4A})"
                ),
            );
            return SONIC_STATUS_INVALID_OUTPUT_FORMAT;
        }
    };

    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());
    match transcoder.transcode_with_preset_and_format(input, quality, output_format) {
        Ok(mut bytes) => {
            let ptr = bytes.as_mut_ptr();
            let len = bytes.len();
            let cap = bytes.capacity();
            std::mem::forget(bytes);

            *out_data_ptr = ptr;
            *out_data_len = len;
            *out_data_cap = cap;
            SONIC_STATUS_OK
        }
        Err(err) => {
            write_error(out_error, err.to_string());
            map_error_to_status(&err)
        }
    }
}

unsafe fn sonic_transcode_to_format_with_bitrate_inner(
    input_ptr: *const u8,
    input_len: usize,
    bitrate_kbps: u32,
    output_format: u32,
    out_data_ptr: *mut *mut u8,
    out_data_len: *mut usize,
    out_data_cap: *mut usize,
    out_error: *mut *mut c_char,
) -> i32 {
    if out_data_ptr.is_null() || out_data_len.is_null() || out_data_cap.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    *out_data_ptr = std::ptr::null_mut();
    *out_data_len = 0;
    *out_data_cap = 0;

    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    if input_ptr.is_null() && input_len > 0 {
        write_error(
            out_error,
            "input_ptr is null while input_len is non-zero".to_string(),
        );
        return SONIC_STATUS_INVALID_ARGS;
    }

    if bitrate_kbps == 0 {
        write_error(out_error, "bitrate_kbps must be > 0".to_string());
        return SONIC_STATUS_INVALID_ARGS;
    }

    let output_format = match parse_output_format(output_format) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}), SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3}), or SONIC_OUTPUT_M4A ({SONIC_OUTPUT_M4A})"
                ),
            );
            return SONIC_STATUS_INVALID_OUTPUT_FORMAT;
        }
    };

    let input = std::slice::from_raw_parts(input_ptr, input_len);
    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());
    match transcoder.transcode_with_bitrate_and_format(input, bitrate_kbps, output_format) {
        Ok(mut bytes) => {
            let ptr = bytes.as_mut_ptr();
            let len = bytes.len();
            let cap = bytes.capacity();
            std::mem::forget(bytes);

            *out_data_ptr = ptr;
            *out_data_len = len;
            *out_data_cap = cap;
            SONIC_STATUS_OK
        }
        Err(err) => {
            write_error(out_error, err.to_string());
            map_error_to_status(&err)
        }
    }
}

/// Transcode an MP3 file to an AAC file with a quality preset.
///
/// - `input_path` and `output_path` must be valid UTF-8 C strings.
/// - On error, `out_error` may contain an allocated C string; release via
///   `sonic_free_c_string`.
///
/// Returns one of SONIC_STATUS_* constants.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_mp3_file_to_aac_file(
    input_path: *const c_char,
    preset: u32,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> i32 {
    sonic_transcode_file_to_format(
        input_path,
        preset,
        SONIC_OUTPUT_AAC,
        output_path,
        out_error,
    )
}

/// Transcode an MP3/WAV/FLAC file to an AAC, M4A, or MP3 file with a quality preset.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_file_to_format(
    input_path: *const c_char,
    preset: u32,
    output_format: u32,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> i32 {
    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    if input_path.is_null() || output_path.is_null() {
        write_error(out_error, "input_path/output_path must not be null".to_string());
        return SONIC_STATUS_INVALID_ARGS;
    }

    let input_path = match CStr::from_ptr(input_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => {
            write_error(out_error, "input_path must not be empty".to_string());
            return SONIC_STATUS_INVALID_ARGS;
        }
        Err(_) => {
            write_error(out_error, "input_path is not valid UTF-8".to_string());
            return SONIC_STATUS_INVALID_ARGS;
        }
    };

    let output_path = match CStr::from_ptr(output_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => {
            write_error(out_error, "output_path must not be empty".to_string());
            return SONIC_STATUS_INVALID_ARGS;
        }
        Err(_) => {
            write_error(out_error, "output_path is not valid UTF-8".to_string());
            return SONIC_STATUS_INVALID_ARGS;
        }
    };

    let quality = match parse_preset(preset) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid preset value {preset}; expected SONIC_PRESET_LOW ({SONIC_PRESET_LOW}), SONIC_PRESET_MEDIUM ({SONIC_PRESET_MEDIUM}), SONIC_PRESET_HIGH ({SONIC_PRESET_HIGH}), or SONIC_PRESET_VERY_HIGH ({SONIC_PRESET_VERY_HIGH})"
                ),
            );
            return SONIC_STATUS_INVALID_PRESET;
        }
    };

    let output_format = match parse_output_format(output_format) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}), SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3}), or SONIC_OUTPUT_M4A ({SONIC_OUTPUT_M4A})"
                ),
            );
            return SONIC_STATUS_INVALID_OUTPUT_FORMAT;
        }
    };

    let input = match fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_error(
                out_error,
                format!("failed to read input file '{input_path}': {err}"),
            );
            return SONIC_STATUS_INVALID_ARGS;
        }
    };

    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());
    let output = match transcoder.transcode_with_preset_and_format(
        &input,
        quality,
        output_format,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_error(out_error, err.to_string());
            return map_error_to_status(&err);
        }
    };

    match fs::write(&output_path, output) {
        Ok(()) => SONIC_STATUS_OK,
        Err(err) => {
            write_error(
                out_error,
                format!("failed to write output file '{output_path}': {err}"),
            );
            SONIC_STATUS_INTERNAL_ERROR
        }
    }
}

/// Transcode an MP3/WAV/FLAC file to an AAC, M4A, or MP3 file with an explicit bitrate.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_file_to_format_with_bitrate(
    input_path: *const c_char,
    bitrate_kbps: u32,
    output_format: u32,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> i32 {
    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    if bitrate_kbps == 0 {
        write_error(out_error, "bitrate_kbps must be > 0".to_string());
        return SONIC_STATUS_INVALID_ARGS;
    }

    let (input_path, output_path) = match parse_paths(input_path, output_path, out_error) {
        Some(paths) => paths,
        None => return SONIC_STATUS_INVALID_ARGS,
    };

    let output_format = match parse_output_format(output_format) {
        Some(v) => v,
        None => {
            write_error(
                out_error,
                format!(
                    "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}), SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3}), or SONIC_OUTPUT_M4A ({SONIC_OUTPUT_M4A})"
                ),
            );
            return SONIC_STATUS_INVALID_OUTPUT_FORMAT;
        }
    };

    let input = match fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_error(
                out_error,
                format!("failed to read input file '{input_path}': {err}"),
            );
            return SONIC_STATUS_INVALID_ARGS;
        }
    };

    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());
    let output = match transcoder.transcode_with_bitrate_and_format(
        &input,
        bitrate_kbps,
        output_format,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_error(out_error, err.to_string());
            return map_error_to_status(&err);
        }
    };

    match fs::write(&output_path, output) {
        Ok(()) => SONIC_STATUS_OK,
        Err(err) => {
            write_error(
                out_error,
                format!("failed to write output file '{output_path}': {err}"),
            );
            SONIC_STATUS_INTERNAL_ERROR
        }
    }
}

/// Probe basic audio properties without producing encoded output.
#[no_mangle]
pub unsafe extern "C" fn sonic_probe_audio(
    input_ptr: *const u8,
    input_len: usize,
    out_info: *mut SonicAudioInfo,
    out_error: *mut *mut c_char,
) -> i32 {
    if out_info.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    *out_info = SonicAudioInfo::empty();

    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    if input_ptr.is_null() && input_len > 0 {
        write_error(
            out_error,
            "input_ptr is null while input_len is non-zero".to_string(),
        );
        return SONIC_STATUS_INVALID_ARGS;
    }

    let input = std::slice::from_raw_parts(input_ptr, input_len);
    match probe::probe(input) {
        Ok(info) => {
            *out_info = SonicAudioInfo::from_audio_info(info);
            SONIC_STATUS_OK
        }
        Err(err) => {
            write_error(out_error, err.to_string());
            map_error_to_status(&err)
        }
    }
}

/// Return compile-time capabilities for the current Sonic build.
#[no_mangle]
pub extern "C" fn sonic_get_capabilities() -> SonicCapabilities {
    SonicCapabilities {
        abi_version: sonic_ffi_abi_version(),
        input_formats: SONIC_CAP_INPUT_MP3 | SONIC_CAP_INPUT_WAV | SONIC_CAP_INPUT_FLAC,
        output_formats: SONIC_CAP_OUTPUT_MP3 | aac_output_capabilities(),
        features: aac_feature_capabilities(),
        preset_count: 4,
    }
}

/// Release a buffer previously returned by `sonic_transcode_mp3_to_aac`.
#[no_mangle]
pub unsafe extern "C" fn sonic_free_buffer(ptr: *mut u8, len: usize, cap: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Vec::from_raw_parts(ptr, len, cap));
}

/// Release an error string previously returned by `sonic_transcode_mp3_to_aac`.
#[no_mangle]
pub unsafe extern "C" fn sonic_free_c_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(CString::from_raw(ptr));
}

/// Returns the ABI version of Sonic FFI.
#[no_mangle]
pub extern "C" fn sonic_ffi_abi_version() -> u32 {
    2
}

fn parse_preset(preset: u32) -> Option<QualityPreset> {
    match preset {
        SONIC_PRESET_LOW => Some(QualityPreset::Low),
        SONIC_PRESET_MEDIUM => Some(QualityPreset::Medium),
        SONIC_PRESET_HIGH => Some(QualityPreset::High),
        SONIC_PRESET_VERY_HIGH => Some(QualityPreset::VeryHigh),
        _ => None,
    }
}

fn parse_output_format(output_format: u32) -> Option<OutputFormat> {
    match output_format {
        SONIC_OUTPUT_AAC => Some(OutputFormat::Aac),
        SONIC_OUTPUT_MP3 => Some(OutputFormat::Mp3),
        SONIC_OUTPUT_M4A => Some(OutputFormat::M4a),
        _ => None,
    }
}

unsafe fn parse_paths(
    input_path: *const c_char,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> Option<(String, String)> {
    if input_path.is_null() || output_path.is_null() {
        write_error(out_error, "input_path/output_path must not be null".to_string());
        return None;
    }

    let input_path = match CStr::from_ptr(input_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => {
            write_error(out_error, "input_path must not be empty".to_string());
            return None;
        }
        Err(_) => {
            write_error(out_error, "input_path is not valid UTF-8".to_string());
            return None;
        }
    };

    let output_path = match CStr::from_ptr(output_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => {
            write_error(out_error, "output_path must not be empty".to_string());
            return None;
        }
        Err(_) => {
            write_error(out_error, "output_path is not valid UTF-8".to_string());
            return None;
        }
    };

    Some((input_path.to_string(), output_path.to_string()))
}

fn write_error(out_error: *mut *mut c_char, message: String) {
    if out_error.is_null() {
        return;
    }

    let sanitized = message.replace('\0', " ");
    let c = CString::new(sanitized).unwrap_or_else(|_| {
        CString::new("sonic error").expect("CString::new on static literal must succeed")
    });
    // SAFETY: caller provided out_error pointer validity is validated by FFI contract.
    unsafe {
        *out_error = c.into_raw();
    }
}

fn map_error_to_status(err: &TranscodeError) -> i32 {
    match err {
        TranscodeError::EmptyBody => SONIC_STATUS_INVALID_ARGS,
        TranscodeError::UnsupportedFormat => SONIC_STATUS_UNSUPPORTED_FORMAT,
        TranscodeError::InvalidPreset(_) => SONIC_STATUS_INVALID_PRESET,
        TranscodeError::InvalidOutputFormat(_) => SONIC_STATUS_INVALID_OUTPUT_FORMAT,
        TranscodeError::Decode(_) => SONIC_STATUS_DECODE_ERROR,
        TranscodeError::Encode(_) => SONIC_STATUS_ENCODE_ERROR,
        TranscodeError::NotImplemented(_) => SONIC_STATUS_NOT_IMPLEMENTED,
    }
}

impl SonicAudioInfo {
    fn empty() -> Self {
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

    fn from_audio_info(info: AudioInfo) -> Self {
        Self {
            input_format: input_format_code(info.input_format),
            sample_rate: info.sample_rate,
            channels: u32::from(info.channels),
            bits_per_sample: u32::from(info.bits_per_sample),
            duration_ms: info.duration_ms,
            total_samples_per_channel: info.total_samples_per_channel,
            has_metadata: u32::from(info.has_metadata),
            has_artwork: u32::from(info.has_artwork),
        }
    }
}

fn input_format_code(input_format: InputFormat) -> u32 {
    match input_format {
        InputFormat::Mp3 => SONIC_INPUT_MP3,
        InputFormat::Wav => SONIC_INPUT_WAV,
        InputFormat::Flac => SONIC_INPUT_FLAC,
    }
}

#[cfg(feature = "aac-fdk")]
fn aac_output_capabilities() -> u32 {
    SONIC_CAP_OUTPUT_AAC | SONIC_CAP_OUTPUT_M4A
}

#[cfg(not(feature = "aac-fdk"))]
fn aac_output_capabilities() -> u32 {
    0
}

#[cfg(feature = "aac-fdk")]
fn aac_feature_capabilities() -> u32 {
    SONIC_CAP_AAC_FDK
}

#[cfg(not(feature = "aac-fdk"))]
fn aac_feature_capabilities() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_build_capabilities() {
        let caps = sonic_get_capabilities();

        assert_eq!(caps.abi_version, 2);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_MP3, 0);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_WAV, 0);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_FLAC, 0);
        assert_ne!(caps.output_formats & SONIC_CAP_OUTPUT_MP3, 0);
        assert_eq!(caps.preset_count, 4);
    }

    #[test]
    fn maps_new_presets_and_outputs() {
        assert_eq!(parse_preset(SONIC_PRESET_HIGH), Some(QualityPreset::High));
        assert_eq!(
            parse_preset(SONIC_PRESET_VERY_HIGH),
            Some(QualityPreset::VeryHigh)
        );
        assert_eq!(parse_output_format(SONIC_OUTPUT_M4A), Some(OutputFormat::M4a));
    }
}
