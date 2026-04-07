#ifndef SONIC_FFI_H
#define SONIC_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Status codes
#define SONIC_STATUS_OK 0
#define SONIC_STATUS_INVALID_ARGS 1
#define SONIC_STATUS_UNSUPPORTED_FORMAT 2
#define SONIC_STATUS_DECODE_ERROR 3
#define SONIC_STATUS_ENCODE_ERROR 4
#define SONIC_STATUS_NOT_IMPLEMENTED 5
#define SONIC_STATUS_INVALID_PRESET 6
#define SONIC_STATUS_INTERNAL_ERROR 7
#define SONIC_STATUS_INVALID_OUTPUT_FORMAT 8

// Presets for sonic_transcode_mp3_to_aac
#define SONIC_PRESET_LOW 0
#define SONIC_PRESET_MEDIUM 1
#define SONIC_OUTPUT_AAC 0
#define SONIC_OUTPUT_MP3 1

// Transcodes MP3 bytes to AAC bytes.
//
// - input_ptr/input_len: input MP3 data
// - preset: SONIC_PRESET_LOW or SONIC_PRESET_MEDIUM
// - out_data_ptr/out_data_len/out_data_cap: allocated output buffer on success
// - out_error: optional allocated C string on error (nullable)
//
// Memory ownership:
// - Free out_data_ptr with sonic_free_buffer(out_data_ptr, out_data_len, out_data_cap)
// - Free out_error with sonic_free_c_string(out_error)
int32_t sonic_transcode_mp3_to_aac(
    const uint8_t* input_ptr,
    size_t input_len,
    uint32_t preset,
    uint8_t** out_data_ptr,
    size_t* out_data_len,
    size_t* out_data_cap,
    char** out_error
);

// Generic transcode API supporting MP3/WAV/FLAC input and AAC/MP3 output.
int32_t sonic_transcode_to_format(
    const uint8_t* input_ptr,
    size_t input_len,
    uint32_t preset,
    uint32_t output_format,
    uint8_t** out_data_ptr,
    size_t* out_data_len,
    size_t* out_data_cap,
    char** out_error
);

// Transcodes an MP3 file path to an AAC file path.
int32_t sonic_transcode_mp3_file_to_aac_file(
    const char* input_path,
    uint32_t preset,
    const char* output_path,
    char** out_error
);

void sonic_free_buffer(uint8_t* ptr, size_t len, size_t cap);
void sonic_free_c_string(char* ptr);
uint32_t sonic_ffi_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif
