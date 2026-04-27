#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:-0.1.0}"
PLATFORM="${PLATFORM:-linux-arm64}"
IMAGE="${IMAGE:-rust:1-bookworm}"
BUILD_AAC="${BUILD_AAC:-1}"
PACKAGE="sonic-transcoder-v${VERSION}-${PLATFORM}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Using Docker image: ${IMAGE}"

docker run --rm \
  --platform linux/arm64 \
  -v "${REPO_ROOT}:/work" \
  -w /work \
  -e VERSION="${VERSION}" \
  -e PLATFORM="${PLATFORM}" \
  -e BUILD_AAC="${BUILD_AAC}" \
  "${IMAGE}" \
  bash -lc '
    set -euo pipefail

    export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

    if ! command -v cargo >/dev/null 2>&1; then
      echo "cargo was not found in ${HOSTNAME}; installing Rust toolchain inside the container..."
      apt-get update
      apt-get install -y --no-install-recommends ca-certificates curl build-essential pkg-config
      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
      export PATH="${HOME}/.cargo/bin:${PATH}"
    fi

    cargo --version
    rustc --version

    export CARGO_TARGET_DIR=target/docker-linux-arm64

    if [ "${BUILD_AAC}" = "1" ]; then
      cargo build --release --features aac-fdk --lib
    else
      cargo build --release --lib
    fi

    PACKAGE="sonic-transcoder-v${VERSION}-${PLATFORM}"
    DIST="dist/${PACKAGE}"

    rm -rf "${DIST}" "dist/${PACKAGE}.tar.gz"
    mkdir -p "${DIST}/include" "${DIST}/lib" "${DIST}/examples/c"

    cp include/sonic_ffi.h "${DIST}/include/"
    cp "${CARGO_TARGET_DIR}/release/libsonic_transcoder.so" "${DIST}/lib/"
    cp examples/c/*.c "${DIST}/examples/c/"
    cp README.md GUIDE.md LICENSE.md "${DIST}/"

    tar -czf "dist/${PACKAGE}.tar.gz" -C dist "${PACKAGE}"
  '

echo "Built dist/${PACKAGE}.tar.gz"
