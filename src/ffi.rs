use std::{
    ffi::{c_char, CStr, CString},
    fs,
};

use crate::{
    audio::{output::OutputFormat, preset::QualityPreset, transcoder::Transcoder},
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

/// Quality presets accepted by `sonic_transcode_mp3_to_aac`.
pub const SONIC_PRESET_LOW: u32 = 0;
pub const SONIC_PRESET_MEDIUM: u32 = 1;
pub const SONIC_OUTPUT_AAC: u32 = 0;
pub const SONIC_OUTPUT_MP3: u32 = 1;

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

/// Transcode MP3/WAV/FLAC bytes to AAC or MP3 bytes.
///
/// Presets:
/// - SONIC_PRESET_LOW
/// - SONIC_PRESET_MEDIUM
///
/// Output formats:
/// - SONIC_OUTPUT_AAC
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
                    "invalid preset value {preset}; expected SONIC_PRESET_LOW ({SONIC_PRESET_LOW}) or SONIC_PRESET_MEDIUM ({SONIC_PRESET_MEDIUM})"
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
                    "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}) or SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3})"
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
                    "invalid preset value {preset}; expected SONIC_PRESET_LOW ({SONIC_PRESET_LOW}) or SONIC_PRESET_MEDIUM ({SONIC_PRESET_MEDIUM})"
                ),
            );
            return SONIC_STATUS_INVALID_PRESET;
        }
    };

    let input = match fs::read(input_path) {
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
        OutputFormat::Aac,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            write_error(out_error, err.to_string());
            return map_error_to_status(&err);
        }
    };

    match fs::write(output_path, output) {
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
    1
}

fn parse_preset(preset: u32) -> Option<QualityPreset> {
    match preset {
        SONIC_PRESET_LOW => Some(QualityPreset::Low),
        SONIC_PRESET_MEDIUM => Some(QualityPreset::Medium),
        _ => None,
    }
}

fn parse_output_format(output_format: u32) -> Option<OutputFormat> {
    match output_format {
        SONIC_OUTPUT_AAC => Some(OutputFormat::Aac),
        SONIC_OUTPUT_MP3 => Some(OutputFormat::Mp3),
        _ => None,
    }
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
