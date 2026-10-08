#!/usr/bin/env bash
# Export the NDK cross toolchain for the cc crate (aarch64-linux-android).
# Must be `source`d. Consumes ANDROID_NDK_HOME or ANDROID_NDK_ROOT.
set -euo pipefail

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$NDK" ]; then
    echo "error: ANDROID_NDK_HOME / ANDROID_NDK_ROOT not set" >&2
    exit 1
fi

TOOLBIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
if [ ! -x "$TOOLBIN/aarch64-linux-android26-clang" ]; then
    echo "error: NDK toolchain not found under $TOOLBIN" >&2
    exit 1
fi

export CC_aarch64_linux_android="$TOOLBIN/aarch64-linux-android26-clang"
export CXX_aarch64_linux_android="$TOOLBIN/aarch64-linux-android26-clang++"
export AR_aarch64_linux_android="$TOOLBIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLBIN/aarch64-linux-android26-clang"
# cc crate: C++ standard library for the android target; both spellings for
# compatibility across cc versions.
export CXXSTLD_aarch64_linux_android="c++_static"
export CXXSTDLIB_aarch64_linux_android="c++_static"

echo "NDK toolchain ready: $NDK"
