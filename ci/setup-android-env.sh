#!/usr/bin/env bash
# Export the NDK cross linker for the aarch64-linux-android Rust target
# (任务板 #77 起 C++/cc 层已删除，无需 CXX/AR/C++ 标准库导出；NDK clang
# 仍承担交叉链接)。Must be `source`d. Consumes ANDROID_NDK_HOME or
# ANDROID_NDK_ROOT.
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
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLBIN/aarch64-linux-android26-clang"

echo "NDK toolchain ready: $NDK"
