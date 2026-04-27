use std::{
    ffi::{c_char, CString},
    fs,
    path::Path,
};

use crate::{
    audio::{preset::QualityPreset, probe, transcoder::Transcoder},
    batch,
    ffi::{
        convert::{
            aac_feature_capabilities, aac_output_capabilities, audio_info_to_ffi,
            batch_options_from_ffi, batch_summary_to_ffi, invalid_output_format_message,
            invalid_preset_message, parse_output_format, parse_paths, parse_preset,
        },
        support::{buffer_from_vec, drop_buffer, map_error_to_status, reset_buffer, write_error},
    },
};

pub mod convert;
pub mod support;
pub mod types;

pub use types::{
    SonicAudioInfo, SonicBatchOptions, SonicBatchResult, SonicBuffer, SonicCapabilities,
    SonicTranscodeOptions, SONIC_CAP_AAC_FDK, SONIC_CAP_INPUT_FLAC, SONIC_CAP_INPUT_MP3,
    SONIC_CAP_INPUT_WAV, SONIC_CAP_OUTPUT_AAC, SONIC_CAP_OUTPUT_M4A, SONIC_CAP_OUTPUT_MP3,
    SONIC_INPUT_FLAC, SONIC_INPUT_MP3, SONIC_INPUT_WAV, SONIC_OUTPUT_AAC, SONIC_OUTPUT_M4A,
    SONIC_OUTPUT_MP3, SONIC_PRESET_HIGH, SONIC_PRESET_LOW, SONIC_PRESET_MEDIUM,
    SONIC_PRESET_VERY_HIGH, SONIC_STATUS_DECODE_ERROR, SONIC_STATUS_ENCODE_ERROR,
    SONIC_STATUS_INTERNAL_ERROR, SONIC_STATUS_INVALID_ARGS, SONIC_STATUS_INVALID_OUTPUT_FORMAT,
    SONIC_STATUS_INVALID_PRESET, SONIC_STATUS_NOT_IMPLEMENTED, SONIC_STATUS_OK,
    SONIC_STATUS_UNSUPPORTED_FORMAT,
};

/// Transcode MP3 bytes to AAC bytes with a quality preset.
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

    *out_data_ptr = std::ptr::null_mut();
    *out_data_len = 0;
    *out_data_cap = 0;

    let mut buffer = SonicBuffer::empty();
    let status = transcode_preset_to_buffer(
        input_ptr,
        input_len,
        preset,
        output_format,
        &mut buffer,
        out_error,
    );

    if status == SONIC_STATUS_OK {
        *out_data_ptr = buffer.ptr;
        *out_data_len = buffer.len;
        *out_data_cap = buffer.cap;
    }

    status
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
    if out_data_ptr.is_null() || out_data_len.is_null() || out_data_cap.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    *out_data_ptr = std::ptr::null_mut();
    *out_data_len = 0;
    *out_data_cap = 0;

    let options = SonicTranscodeOptions {
        output_format,
        preset: SONIC_PRESET_MEDIUM,
        bitrate_kbps,
        reserved: 0,
    };
    let mut buffer = SonicBuffer::empty();
    let status = sonic_transcode(input_ptr, input_len, &options, &mut buffer, out_error);

    if status == SONIC_STATUS_OK {
        *out_data_ptr = buffer.ptr;
        *out_data_len = buffer.len;
        *out_data_cap = buffer.cap;
    }

    status
}

/// Return default transcode options for the options-based API.
#[no_mangle]
pub extern "C" fn sonic_default_transcode_options() -> SonicTranscodeOptions {
    SonicTranscodeOptions {
        output_format: SONIC_OUTPUT_MP3,
        preset: SONIC_PRESET_MEDIUM,
        bitrate_kbps: 0,
        reserved: 0,
    }
}

/// Return default directory batch options.
#[no_mangle]
pub extern "C" fn sonic_default_batch_options() -> SonicBatchOptions {
    SonicBatchOptions {
        transcode: sonic_default_transcode_options(),
        workers: 0,
        reserved: 0,
    }
}

/// Transcode bytes using a compact options struct and single output buffer.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode(
    input_ptr: *const u8,
    input_len: usize,
    options: *const SonicTranscodeOptions,
    out_buffer: *mut SonicBuffer,
    out_error: *mut *mut c_char,
) -> i32 {
    if out_buffer.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    reset_buffer(out_buffer);

    let options = if options.is_null() {
        sonic_default_transcode_options()
    } else {
        *options
    };

    if options.bitrate_kbps > 0 {
        transcode_bitrate_to_buffer(
            input_ptr,
            input_len,
            options.bitrate_kbps,
            options.output_format,
            out_buffer,
            out_error,
        )
    } else {
        transcode_preset_to_buffer(
            input_ptr,
            input_len,
            options.preset,
            options.output_format,
            out_buffer,
            out_error,
        )
    }
}

/// Compatibility helper: transcodes an MP3 file path to an AAC file path.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_mp3_file_to_aac_file(
    input_path: *const c_char,
    preset: u32,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> i32 {
    sonic_transcode_file_to_format(input_path, preset, SONIC_OUTPUT_AAC, output_path, out_error)
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
    let options = SonicTranscodeOptions {
        output_format,
        preset,
        bitrate_kbps: 0,
        reserved: 0,
    };
    sonic_transcode_file(input_path, &options, output_path, out_error)
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
    let options = SonicTranscodeOptions {
        output_format,
        preset: SONIC_PRESET_MEDIUM,
        bitrate_kbps,
        reserved: 0,
    };
    sonic_transcode_file(input_path, &options, output_path, out_error)
}

/// Transcode a file using a compact options struct.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_file(
    input_path: *const c_char,
    options: *const SonicTranscodeOptions,
    output_path: *const c_char,
    out_error: *mut *mut c_char,
) -> i32 {
    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    let (input_path, output_path) = match parse_paths(input_path, output_path) {
        Ok(paths) => paths,
        Err(message) => {
            write_error(out_error, message);
            return SONIC_STATUS_INVALID_ARGS;
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

    let options = if options.is_null() {
        sonic_default_transcode_options()
    } else {
        *options
    };

    let output = match transcode_bytes(&input, options) {
        Ok(bytes) => bytes,
        Err((status, message)) => {
            write_error(out_error, message);
            return status;
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

/// Transcode all supported audio files in a directory tree.
#[no_mangle]
pub unsafe extern "C" fn sonic_transcode_directory(
    input_dir: *const c_char,
    output_dir: *const c_char,
    options: *const SonicBatchOptions,
    out_result: *mut SonicBatchResult,
    out_error: *mut *mut c_char,
) -> i32 {
    if out_result.is_null() {
        return SONIC_STATUS_INVALID_ARGS;
    }

    *out_result = SonicBatchResult::empty();

    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    let (input_dir, output_dir) = match parse_paths(input_dir, output_dir) {
        Ok(paths) => paths,
        Err(message) => {
            write_error(out_error, message);
            return SONIC_STATUS_INVALID_ARGS;
        }
    };

    let options = if options.is_null() {
        sonic_default_batch_options()
    } else {
        *options
    };

    let options = match batch_options_from_ffi(options) {
        Ok(options) => options,
        Err((status, message)) => {
            write_error(out_error, message);
            return status;
        }
    };

    match batch::transcode_directory(Path::new(&input_dir), Path::new(&output_dir), options) {
        Ok(summary) => {
            *out_result = batch_summary_to_ffi(summary);
            SONIC_STATUS_OK
        }
        Err(message) => {
            write_error(out_error, message);
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

    let input = match input_slice(input_ptr, input_len) {
        Ok(input) => input,
        Err(message) => {
            write_error(out_error, message);
            return SONIC_STATUS_INVALID_ARGS;
        }
    };
    match probe::probe(input) {
        Ok(info) => {
            *out_info = audio_info_to_ffi(info);
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

/// Release a buffer previously returned through the legacy pointer/len/cap API.
#[no_mangle]
pub unsafe extern "C" fn sonic_free_buffer(ptr: *mut u8, len: usize, cap: usize) {
    drop_buffer(SonicBuffer { ptr, len, cap });
}

/// Release a buffer previously returned through `sonic_transcode`.
#[no_mangle]
pub unsafe extern "C" fn sonic_free_output_buffer(buffer: *mut SonicBuffer) {
    if buffer.is_null() {
        return;
    }
    let owned = *buffer;
    *buffer = SonicBuffer::empty();
    drop_buffer(owned);
}

/// Release an error string previously returned by Sonic.
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
    4
}

unsafe fn transcode_preset_to_buffer(
    input_ptr: *const u8,
    input_len: usize,
    preset: u32,
    output_format: u32,
    out_buffer: *mut SonicBuffer,
    out_error: *mut *mut c_char,
) -> i32 {
    let options = SonicTranscodeOptions {
        output_format,
        preset,
        bitrate_kbps: 0,
        reserved: 0,
    };
    transcode_options_to_buffer(input_ptr, input_len, options, out_buffer, out_error)
}

unsafe fn transcode_bitrate_to_buffer(
    input_ptr: *const u8,
    input_len: usize,
    bitrate_kbps: u32,
    output_format: u32,
    out_buffer: *mut SonicBuffer,
    out_error: *mut *mut c_char,
) -> i32 {
    let options = SonicTranscodeOptions {
        output_format,
        preset: SONIC_PRESET_MEDIUM,
        bitrate_kbps,
        reserved: 0,
    };
    transcode_options_to_buffer(input_ptr, input_len, options, out_buffer, out_error)
}

unsafe fn transcode_options_to_buffer(
    input_ptr: *const u8,
    input_len: usize,
    options: SonicTranscodeOptions,
    out_buffer: *mut SonicBuffer,
    out_error: *mut *mut c_char,
) -> i32 {
    if !out_error.is_null() {
        *out_error = std::ptr::null_mut();
    }

    let input = match input_slice(input_ptr, input_len) {
        Ok(input) => input,
        Err(message) => {
            write_error(out_error, message);
            return SONIC_STATUS_INVALID_ARGS;
        }
    };
    let output = match transcode_bytes(input, options) {
        Ok(bytes) => bytes,
        Err((status, message)) => {
            write_error(out_error, message);
            return status;
        }
    };

    *out_buffer = buffer_from_vec(output);
    SONIC_STATUS_OK
}

fn transcode_bytes(input: &[u8], options: SonicTranscodeOptions) -> Result<Vec<u8>, (i32, String)> {
    let output_format = parse_output_format(options.output_format).ok_or_else(|| {
        (
            SONIC_STATUS_INVALID_OUTPUT_FORMAT,
            invalid_output_format_message(options.output_format),
        )
    })?;

    let transcoder = Transcoder::new(QualityPreset::Medium.bitrate_kbps());

    let result = if options.bitrate_kbps > 0 {
        transcoder.transcode_with_bitrate_and_format(input, options.bitrate_kbps, output_format)
    } else {
        let quality = parse_preset(options.preset).ok_or_else(|| {
            (
                SONIC_STATUS_INVALID_PRESET,
                invalid_preset_message(options.preset),
            )
        })?;
        transcoder.transcode_with_preset_and_format(input, quality, output_format)
    };

    result.map_err(|err| (map_error_to_status(&err), err.to_string()))
}

unsafe fn input_slice<'a>(
    input_ptr: *const u8,
    input_len: usize,
) -> Result<&'a [u8], &'static str> {
    if input_ptr.is_null() {
        if input_len == 0 {
            Ok(&[])
        } else {
            Err("input_ptr is null while input_len is non-zero")
        }
    } else {
        Ok(std::slice::from_raw_parts(input_ptr, input_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_build_capabilities() {
        let caps = sonic_get_capabilities();

        assert_eq!(caps.abi_version, 4);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_MP3, 0);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_WAV, 0);
        assert_ne!(caps.input_formats & SONIC_CAP_INPUT_FLAC, 0);
        assert_ne!(caps.output_formats & SONIC_CAP_OUTPUT_MP3, 0);
        assert_eq!(caps.preset_count, 4);
    }

    #[test]
    fn default_options_are_stable_and_simple() {
        let options = sonic_default_transcode_options();

        assert_eq!(options.output_format, SONIC_OUTPUT_MP3);
        assert_eq!(options.preset, SONIC_PRESET_MEDIUM);
        assert_eq!(options.bitrate_kbps, 0);
    }

    #[test]
    fn default_batch_options_use_auto_workers() {
        let options = sonic_default_batch_options();

        assert_eq!(options.transcode.output_format, SONIC_OUTPUT_MP3);
        assert_eq!(options.transcode.preset, SONIC_PRESET_MEDIUM);
        assert_eq!(options.workers, 0);
    }

    #[test]
    fn maps_new_presets_and_outputs() {
        assert_eq!(parse_preset(SONIC_PRESET_HIGH), Some(QualityPreset::High));
        assert_eq!(
            parse_preset(SONIC_PRESET_VERY_HIGH),
            Some(QualityPreset::VeryHigh)
        );
        assert_eq!(
            parse_output_format(SONIC_OUTPUT_M4A),
            Some(crate::audio::output::OutputFormat::M4a)
        );
    }
}
