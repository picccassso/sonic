use std::ffi::{c_char, CStr};

use crate::{
    audio::{
        detect::InputFormat,
        output::OutputFormat,
        preset::QualityPreset,
        probe::AudioInfo,
    },
    batch::{BatchTranscodeOptions, BatchTranscodeSummary},
    ffi::types::*,
};

pub fn parse_preset(preset: u32) -> Option<QualityPreset> {
    match preset {
        SONIC_PRESET_LOW => Some(QualityPreset::Low),
        SONIC_PRESET_MEDIUM => Some(QualityPreset::Medium),
        SONIC_PRESET_HIGH => Some(QualityPreset::High),
        SONIC_PRESET_VERY_HIGH => Some(QualityPreset::VeryHigh),
        _ => None,
    }
}

pub fn parse_output_format(output_format: u32) -> Option<OutputFormat> {
    match output_format {
        SONIC_OUTPUT_AAC => Some(OutputFormat::Aac),
        SONIC_OUTPUT_MP3 => Some(OutputFormat::Mp3),
        SONIC_OUTPUT_M4A => Some(OutputFormat::M4a),
        _ => None,
    }
}

pub unsafe fn parse_paths(
    input_path: *const c_char,
    output_path: *const c_char,
) -> Result<(String, String), &'static str> {
    if input_path.is_null() || output_path.is_null() {
        return Err("input_path/output_path must not be null");
    }

    let input_path = match CStr::from_ptr(input_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => return Err("input_path must not be empty"),
        Err(_) => return Err("input_path is not valid UTF-8"),
    };

    let output_path = match CStr::from_ptr(output_path).to_str() {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => return Err("output_path must not be empty"),
        Err(_) => return Err("output_path is not valid UTF-8"),
    };

    Ok((input_path.to_string(), output_path.to_string()))
}

pub fn audio_info_to_ffi(info: AudioInfo) -> SonicAudioInfo {
    SonicAudioInfo {
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

pub fn batch_options_from_ffi(options: SonicBatchOptions) -> Result<BatchTranscodeOptions, (i32, String)> {
    let output_format = parse_output_format(options.transcode.output_format).ok_or_else(|| {
        (
            SONIC_STATUS_INVALID_OUTPUT_FORMAT,
            invalid_output_format_message(options.transcode.output_format),
        )
    })?;

    let preset = parse_preset(options.transcode.preset).ok_or_else(|| {
        (
            SONIC_STATUS_INVALID_PRESET,
            invalid_preset_message(options.transcode.preset),
        )
    })?;

    Ok(BatchTranscodeOptions {
        output_format,
        preset,
        bitrate_kbps: if options.transcode.bitrate_kbps == 0 {
            None
        } else {
            Some(options.transcode.bitrate_kbps)
        },
        workers: options.workers as usize,
    })
}

pub fn batch_summary_to_ffi(summary: BatchTranscodeSummary) -> SonicBatchResult {
    SonicBatchResult {
        files_total: summary.files_total,
        files_completed: summary.files_completed,
        files_failed: summary.files_failed,
        input_bytes: summary.input_bytes,
        output_bytes: summary.output_bytes,
        workers_used: summary.workers_used as u32,
    }
}

pub fn invalid_preset_message(preset: u32) -> String {
    format!(
        "invalid preset value {preset}; expected SONIC_PRESET_LOW ({SONIC_PRESET_LOW}), SONIC_PRESET_MEDIUM ({SONIC_PRESET_MEDIUM}), SONIC_PRESET_HIGH ({SONIC_PRESET_HIGH}), or SONIC_PRESET_VERY_HIGH ({SONIC_PRESET_VERY_HIGH})"
    )
}

pub fn invalid_output_format_message(output_format: u32) -> String {
    format!(
        "invalid output format value {output_format}; expected SONIC_OUTPUT_AAC ({SONIC_OUTPUT_AAC}), SONIC_OUTPUT_MP3 ({SONIC_OUTPUT_MP3}), or SONIC_OUTPUT_M4A ({SONIC_OUTPUT_M4A})"
    )
}

fn input_format_code(input_format: InputFormat) -> u32 {
    match input_format {
        InputFormat::Mp3 => SONIC_INPUT_MP3,
        InputFormat::Wav => SONIC_INPUT_WAV,
        InputFormat::Flac => SONIC_INPUT_FLAC,
    }
}

#[cfg(feature = "aac-fdk")]
pub fn aac_output_capabilities() -> u32 {
    SONIC_CAP_OUTPUT_AAC | SONIC_CAP_OUTPUT_M4A
}

#[cfg(not(feature = "aac-fdk"))]
pub fn aac_output_capabilities() -> u32 {
    0
}

#[cfg(feature = "aac-fdk")]
pub fn aac_feature_capabilities() -> u32 {
    SONIC_CAP_AAC_FDK
}

#[cfg(not(feature = "aac-fdk"))]
pub fn aac_feature_capabilities() -> u32 {
    0
}
