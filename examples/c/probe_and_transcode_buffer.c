#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "../../include/sonic_ffi.h"

static uint8_t* read_file(const char* path, size_t* len) {
    FILE* file = fopen(path, "rb");
    if (!file) {
        return NULL;
    }

    fseek(file, 0, SEEK_END);
    long size = ftell(file);
    fseek(file, 0, SEEK_SET);

    if (size < 0) {
        fclose(file);
        return NULL;
    }

    uint8_t* bytes = (uint8_t*)malloc((size_t)size);
    if (!bytes) {
        fclose(file);
        return NULL;
    }

    if (fread(bytes, 1, (size_t)size, file) != (size_t)size) {
        free(bytes);
        fclose(file);
        return NULL;
    }

    fclose(file);
    *len = (size_t)size;
    return bytes;
}

int main(int argc, char** argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <input-audio>\n", argv[0]);
        return 2;
    }

    size_t input_len = 0;
    uint8_t* input = read_file(argv[1], &input_len);
    if (!input) {
        fprintf(stderr, "failed to read input\n");
        return 1;
    }

    char* error = NULL;
    SonicAudioInfo info = {0};
    int32_t status = sonic_probe_audio(input, input_len, &info, &error);
    if (status != SONIC_STATUS_OK) {
        fprintf(stderr, "probe failed (%d): %s\n", status, error ? error : "unknown error");
        sonic_free_c_string(error);
        free(input);
        return 1;
    }

    printf("sample_rate=%u channels=%u duration_ms=%llu\n",
           info.sample_rate,
           info.channels,
           (unsigned long long)info.duration_ms);

    SonicTranscodeOptions options = sonic_default_transcode_options();
    options.output_format = SONIC_OUTPUT_MP3;
    options.bitrate_kbps = 192;

    SonicBuffer output = {0};
    status = sonic_transcode(input, input_len, &options, &output, &error);
    free(input);

    if (status != SONIC_STATUS_OK) {
        fprintf(stderr, "transcode failed (%d): %s\n", status, error ? error : "unknown error");
        sonic_free_c_string(error);
        return 1;
    }

    printf("encoded_bytes=%zu\n", output.len);
    sonic_free_output_buffer(&output);
    return 0;
}
