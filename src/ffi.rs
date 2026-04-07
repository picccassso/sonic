use std::ffi::{c_char, CString};

use crate::{
    audio::{preset::QualityPreset, transcoder::Transcoder},
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
/// FFI status: internal failure.
pub const SONIC_STATUS_INTERNAL_ERROR: i32 = 7;

/// Quality presets accepted by `sonic_transcode_mp3_to_aac`.
pub const SONIC_PRESET_LOW: u32 = 0;
pub const SONIC_PRESET_MEDIUM: u32 = 1;

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

    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());
    match transcoder.transcode_with_preset(input, quality) {
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
        TranscodeError::Decode(_) => SONIC_STATUS_DECODE_ERROR,
        TranscodeError::Encode(_) => SONIC_STATUS_ENCODE_ERROR,
        TranscodeError::NotImplemented(_) => SONIC_STATUS_NOT_IMPLEMENTED,
    }
}
