use std::ffi::{c_char, CString};

use crate::{errors::TranscodeError, ffi::types::*};

pub fn write_error(out_error: *mut *mut c_char, message: impl Into<String>) {
    if out_error.is_null() {
        return;
    }

    let sanitized = message.into().replace('\0', " ");
    let c = CString::new(sanitized).unwrap_or_else(|_| {
        CString::new("sonic error").expect("CString::new on static literal must succeed")
    });
    unsafe {
        *out_error = c.into_raw();
    }
}

pub fn map_error_to_status(err: &TranscodeError) -> i32 {
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

pub fn buffer_from_vec(mut bytes: Vec<u8>) -> SonicBuffer {
    let buffer = SonicBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
        cap: bytes.capacity(),
    };
    std::mem::forget(bytes);
    buffer
}

pub unsafe fn drop_buffer(buffer: SonicBuffer) {
    if buffer.ptr.is_null() {
        return;
    }
    drop(Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.cap));
}

pub unsafe fn reset_buffer(out_buffer: *mut SonicBuffer) {
    if !out_buffer.is_null() {
        *out_buffer = SonicBuffer::empty();
    }
}
