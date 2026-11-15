#!/usr/bin/env bash
# 注入 MSVC 构建环境（等价于 vcvars64.bat）。
# 用法： source scripts/msvc-env.sh && cargo build
#
# 说明：本机 VS Build Tools 装在 Program Files (x86)，路径全为 ASCII，
# 因此可以直接硬编码，避免 cmd.exe 的中文编码问题。
# 若将来升级 MSVC 版本，改这里的 14.44.35207 与 10.0.26100.0 即可。

MSVC_VER='14.44.35207'
SDK_VER='10.0.26100.0'
VS_ROOT='C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools'
SDK_ROOT='C:\Program Files (x86)\Windows Kits\10'

export MSYS2_ARG_CONV_EXCL='*'

export INCLUDE="${VS_ROOT}\\VC\\Tools\\MSVC\\${MSVC_VER}\\include;${SDK_ROOT}\\Include\\${SDK_VER}\\ucrt;${SDK_ROOT}\\Include\\${SDK_VER}\\um;${SDK_ROOT}\\Include\\${SDK_VER}\\shared;${SDK_ROOT}\\Include\\${SDK_VER}\\winrt"

export LIB="${VS_ROOT}\\VC\\Tools\\MSVC\\${MSVC_VER}\\lib\\x64;${SDK_ROOT}\\Lib\\${SDK_VER}\\ucrt\\x64;${SDK_ROOT}\\Lib\\${SDK_VER}\\um\\x64"

export PATH="/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/${MSVC_VER}/bin/Hostx64/x64:/c/Program Files (x86)/Windows Kits/10/bin/${SDK_VER}/x64:/c/Users/Yu Luo/.cargo/bin:$PATH"

echo "[msvc-env] INCLUDE/LIB/PATH ready (MSVC ${MSVC_VER}, SDK ${SDK_VER})"
