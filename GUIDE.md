# Sonic Integration Guide

Sonic is designed to be embedded through a small C ABI. The Rust code builds a shared library, and host applications call the exported functions from C, C++, Swift, C#, Kotlin/JNI, Node.js, Python, or any runtime that can load a C shared library.

## Build Sonic

```bash
cargo build --release --features aac-fdk --lib
```

Output library names:

- macOS: `target/release/libsonic_transcoder.dylib`
- Linux: `target/release/libsonic_transcoder.so`
- Windows: `target/release/sonic_transcoder.dll`

Include the public header:

```c
#include "include/sonic_ffi.h"
```

## Recommended C API

Prefer the options-based API for new integrations:

```c
SonicTranscodeOptions options = sonic_default_transcode_options();
options.output_format = SONIC_OUTPUT_M4A;
options.preset = SONIC_PRESET_HIGH;

SonicBuffer output = {0};
char* error = NULL;

int32_t status = sonic_transcode(input_bytes, input_len, &options, &output, &error);
if (status == SONIC_STATUS_OK) {
    // Use output.ptr and output.len.
    sonic_free_output_buffer(&output);
} else {
    // Read error, then free it.
    sonic_free_c_string(error);
}
```

Use `bitrate_kbps` when you want an exact bitrate instead of a preset:

```c
SonicTranscodeOptions options = sonic_default_transcode_options();
options.output_format = SONIC_OUTPUT_MP3;
options.bitrate_kbps = 192;
```

## File-To-File C API

```c
SonicTranscodeOptions options = sonic_default_transcode_options();
options.output_format = SONIC_OUTPUT_M4A;
options.preset = SONIC_PRESET_HIGH;

char* error = NULL;
int32_t status = sonic_transcode_file("input.mp3", &options, "output.m4a", &error);
if (status != SONIC_STATUS_OK) {
    fprintf(stderr, "Sonic failed: %s\n", error ? error : "unknown error");
    sonic_free_c_string(error);
}
```

## Directory Batch API

Use the batch API when Sonic should manage a worker pool for a whole folder. `workers = 0` lets Sonic choose from available parallelism; set it explicitly when you want predictable resource usage.

```c
SonicBatchOptions batch = sonic_default_batch_options();
batch.transcode.output_format = SONIC_OUTPUT_M4A;
batch.transcode.preset = SONIC_PRESET_LOW;
batch.workers = 10;

SonicBatchResult result = {0};
char* error = NULL;

int32_t status = sonic_transcode_directory("Music Folder", "Output Folder", &batch, &result, &error);
if (status != SONIC_STATUS_OK) {
    fprintf(stderr, "batch failed: %s\n", error ? error : "unknown error");
    sonic_free_c_string(error);
}

printf("completed=%llu failed=%llu workers=%u\n",
       (unsigned long long)result.files_completed,
       (unsigned long long)result.files_failed,
       result.workers_used);
```

## Probe Before Transcoding

```c
SonicAudioInfo info = {0};
char* error = NULL;

int32_t status = sonic_probe_audio(input_bytes, input_len, &info, &error);
if (status == SONIC_STATUS_OK) {
    printf("sample_rate=%u channels=%u duration_ms=%llu\n",
           info.sample_rate,
           info.channels,
           (unsigned long long)info.duration_ms);
} else {
    sonic_free_c_string(error);
}
```

## C#

.NET can call Sonic with P/Invoke. Current .NET guidance treats C as the stable ABI target for native interop. For .NET 7+, `LibraryImport` is preferred for source-generated interop; `DllImport` is still useful and widely supported.

```csharp
using System;
using System.Runtime.InteropServices;

internal static partial class SonicNative
{
    private const string Library = "sonic_transcoder";

    [StructLayout(LayoutKind.Sequential)]
    internal struct SonicBuffer
    {
        public IntPtr Ptr;
        public nuint Len;
        public nuint Cap;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct SonicTranscodeOptions
    {
        public uint OutputFormat;
        public uint Preset;
        public uint BitrateKbps;
        public uint Reserved;
    }

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SonicTranscodeOptions sonic_default_transcode_options();

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int sonic_transcode(
        byte[] input,
        nuint inputLen,
        ref SonicTranscodeOptions options,
        out SonicBuffer output,
        out IntPtr error);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void sonic_free_output_buffer(ref SonicBuffer buffer);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void sonic_free_c_string(IntPtr ptr);
}
```

Usage:

```csharp
var options = SonicNative.sonic_default_transcode_options();
options.OutputFormat = 2; // SONIC_OUTPUT_M4A
options.Preset = 2;       // SONIC_PRESET_HIGH

int status = SonicNative.sonic_transcode(
    inputBytes,
    (nuint)inputBytes.Length,
    ref options,
    out var output,
    out var error);

if (status != 0)
{
    string message = Marshal.PtrToStringAnsi(error) ?? "unknown error";
    SonicNative.sonic_free_c_string(error);
    throw new InvalidOperationException(message);
}

byte[] encoded = new byte[(int)output.Len];
Marshal.Copy(output.Ptr, encoded, 0, encoded.Length);
SonicNative.sonic_free_output_buffer(ref output);
```

## Swift

For Apple platforms, add `include/sonic_ffi.h` to a module map or bridging header, then link `libsonic_transcoder.dylib`.

```swift
var options = sonic_default_transcode_options()
options.output_format = SONIC_OUTPUT_M4A
options.preset = SONIC_PRESET_HIGH

var output = SonicBuffer()
var error: UnsafeMutablePointer<CChar>?

let status = inputData.withUnsafeBytes { input in
    sonic_transcode(
        input.bindMemory(to: UInt8.self).baseAddress,
        input.count,
        &options,
        &output,
        &error
    )
}

guard status == SONIC_STATUS_OK else {
    let message = error.map { String(cString: $0) } ?? "unknown error"
    sonic_free_c_string(error)
    throw SonicError.transcodeFailed(message)
}

let encoded = Data(bytes: output.ptr, count: output.len)
sonic_free_output_buffer(&output)
```

## Kotlin / Java

Use JNI when integrating directly with Android or JVM software. Keep the JNI layer thin: convert Java/Kotlin byte arrays to native pointers, call `sonic_transcode`, copy the result back, and always release `SonicBuffer`.

```cpp
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_example_Sonic_transcode(JNIEnv* env, jclass, jbyteArray input) {
    jsize input_len = env->GetArrayLength(input);
    jbyte* input_ptr = env->GetByteArrayElements(input, nullptr);

    SonicTranscodeOptions options = sonic_default_transcode_options();
    options.output_format = SONIC_OUTPUT_M4A;
    options.preset = SONIC_PRESET_HIGH;

    SonicBuffer output = {0};
    char* error = nullptr;
    int32_t status = sonic_transcode(
        reinterpret_cast<const uint8_t*>(input_ptr),
        static_cast<size_t>(input_len),
        &options,
        &output,
        &error);

    env->ReleaseByteArrayElements(input, input_ptr, JNI_ABORT);

    if (status != SONIC_STATUS_OK) {
        sonic_free_c_string(error);
        return nullptr;
    }

    jbyteArray result = env->NewByteArray(static_cast<jsize>(output.len));
    env->SetByteArrayRegion(result, 0, static_cast<jsize>(output.len),
                            reinterpret_cast<jbyte*>(output.ptr));
    sonic_free_output_buffer(&output);
    return result;
}
```

## Node.js

For Node or Electron, prefer a small N-API addon over loading the C ABI directly from application code. The addon should expose a JavaScript-friendly API and keep ownership rules inside native code.

```cpp
Napi::Value Transcode(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();
    auto input = info[0].As<Napi::Buffer<uint8_t>>();

    SonicTranscodeOptions options = sonic_default_transcode_options();
    options.output_format = SONIC_OUTPUT_M4A;
    options.preset = SONIC_PRESET_HIGH;

    SonicBuffer output = {0};
    char* error = nullptr;
    int32_t status = sonic_transcode(input.Data(), input.Length(), &options, &output, &error);
    if (status != SONIC_STATUS_OK) {
        std::string message = error ? error : "unknown error";
        sonic_free_c_string(error);
        throw Napi::Error::New(env, message);
    }

    auto result = Napi::Buffer<uint8_t>::Copy(env, output.ptr, output.len);
    sonic_free_output_buffer(&output);
    return result;
}
```

## Python

Python can use `ctypes` for local tools or tests. For production packaging, a small extension module is usually cleaner.

```python
import ctypes

sonic = ctypes.CDLL("./libsonic_transcoder.dylib")

class SonicBuffer(ctypes.Structure):
    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
        ("cap", ctypes.c_size_t),
    ]

class SonicTranscodeOptions(ctypes.Structure):
    _fields_ = [
        ("output_format", ctypes.c_uint32),
        ("preset", ctypes.c_uint32),
        ("bitrate_kbps", ctypes.c_uint32),
        ("reserved", ctypes.c_uint32),
    ]

sonic.sonic_default_transcode_options.restype = SonicTranscodeOptions
sonic.sonic_transcode.argtypes = [
    ctypes.POINTER(ctypes.c_uint8),
    ctypes.c_size_t,
    ctypes.POINTER(SonicTranscodeOptions),
    ctypes.POINTER(SonicBuffer),
    ctypes.POINTER(ctypes.c_char_p),
]
sonic.sonic_transcode.restype = ctypes.c_int32

data = open("input.mp3", "rb").read()
input_buf = (ctypes.c_uint8 * len(data)).from_buffer_copy(data)

options = sonic.sonic_default_transcode_options()
options.output_format = 2
options.preset = 2

output = SonicBuffer()
error = ctypes.c_char_p()
status = sonic.sonic_transcode(input_buf, len(data), ctypes.byref(options), ctypes.byref(output), ctypes.byref(error))

if status != 0:
    message = error.value.decode() if error.value else "unknown error"
    sonic.sonic_free_c_string(error)
    raise RuntimeError(message)

encoded = ctypes.string_at(output.ptr, output.len)
sonic.sonic_free_output_buffer(ctypes.byref(output))
```

## Packaging Tips

Recommended distribution layout:

```text
dist/
  include/
    sonic_ffi.h
  lib/
    libsonic_transcoder.dylib
    libsonic_transcoder.so
    sonic_transcoder.dll
```

Host apps should:

- call `sonic_get_capabilities()` at startup
- check `sonic_ffi_abi_version()`
- keep `sonic_ffi.h` and the shared library from the same build
- free every `SonicBuffer` with `sonic_free_output_buffer`
- free every error string with `sonic_free_c_string`
- avoid exposing raw Sonic pointers outside the language wrapper

## Existing Examples

- `examples/c/transcode_file.c`
- `examples/c/probe_and_transcode_buffer.c`
