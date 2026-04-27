#include <stdio.h>

#include "../../include/sonic_ffi.h"

int main(int argc, char** argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s <input> <output.m4a>\n", argv[0]);
        return 2;
    }

    SonicTranscodeOptions options = sonic_default_transcode_options();
    options.output_format = SONIC_OUTPUT_M4A;
    options.preset = SONIC_PRESET_HIGH;

    char* error = NULL;
    int32_t status = sonic_transcode_file(argv[1], &options, argv[2], &error);
    if (status != SONIC_STATUS_OK) {
        fprintf(stderr, "sonic failed (%d): %s\n", status, error ? error : "unknown error");
        sonic_free_c_string(error);
        return 1;
    }

    return 0;
}
