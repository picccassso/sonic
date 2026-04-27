#include <stdio.h>
#include <stdlib.h>

#include "../../include/sonic_ffi.h"

int main(int argc, char** argv) {
    if (argc != 4) {
        fprintf(stderr, "usage: %s <input-dir> <output-dir> <workers>\n", argv[0]);
        return 2;
    }

    SonicBatchOptions options = sonic_default_batch_options();
    options.transcode.output_format = SONIC_OUTPUT_M4A;
    options.transcode.preset = SONIC_PRESET_LOW;
    options.workers = (uint32_t)atoi(argv[3]);

    SonicBatchResult result = {0};
    char* error = NULL;
    int32_t status = sonic_transcode_directory(argv[1], argv[2], &options, &result, &error);
    if (status != SONIC_STATUS_OK) {
        fprintf(stderr, "sonic failed (%d): %s\n", status, error ? error : "unknown error");
        sonic_free_c_string(error);
        return 1;
    }

    printf("completed=%llu failed=%llu workers=%u input_bytes=%llu output_bytes=%llu\n",
           (unsigned long long)result.files_completed,
           (unsigned long long)result.files_failed,
           result.workers_used,
           (unsigned long long)result.input_bytes,
           (unsigned long long)result.output_bytes);

    return result.files_failed == 0 ? 0 : 1;
}
