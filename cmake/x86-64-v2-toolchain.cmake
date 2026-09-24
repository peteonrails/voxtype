# Toolchain file for the baseline (x86-64-v2) variant.
#
# WHY A TOOLCHAIN FILE: the baseline build must constrain the C/C++ half of
# the binary (ggml/whisper.cpp, built by whisper-rs-sys through the cmake
# crate), and there is no environment variable that reaches ggml's CMake
# options. whisper-rs-sys's build.rs forwards only feature defines (CUDA,
# HIP, Vulkan), and CMake itself does not read GGML_* or CMAKE_C_FLAGS from
# the environment. Setting those as ENV in a Dockerfile does nothing - that
# is exactly how #740 shipped: ggml fell back to its GGML_NATIVE=ON default,
# ran -march=native on an AVX2+BMI2 build host, and the "x86-64-v2" baseline
# binary SIGILLed on shrx (BMI2) at first inference on real Ivy Bridge.
#
# CMake >= 3.21 reads the CMAKE_TOOLCHAIN_FILE environment variable, so this
# file is the one reliable injection point for every CMake sub-build in the
# dependency tree. Dockerfile.baseline sets:
#   ENV CMAKE_TOOLCHAIN_FILE=/build/cmake/x86-64-v2-toolchain.cmake
#
# FORCE is required: these are option() declarations in ggml, and a plain
# set() would be overwritten when the option is declared.

set(CMAKE_C_FLAGS_INIT   "-march=x86-64-v2")
set(CMAKE_CXX_FLAGS_INIT "-march=x86-64-v2")

# Never probe the build host's CPU.
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)

# Everything above the x86-64-v2 floor (SSE4.2 + POPCNT), explicitly off.
# Ivy Bridge has AVX and F16C, but the variant promises v2 (Nehalem 2008+),
# so those stay off too.
set(GGML_SSE42       ON  CACHE BOOL "" FORCE)
set(GGML_AVX         OFF CACHE BOOL "" FORCE)
set(GGML_AVX2        OFF CACHE BOOL "" FORCE)
set(GGML_FMA         OFF CACHE BOOL "" FORCE)
set(GGML_F16C        OFF CACHE BOOL "" FORCE)
set(GGML_BMI2        OFF CACHE BOOL "" FORCE)
set(GGML_AVX_VNNI    OFF CACHE BOOL "" FORCE)
set(GGML_AVX512      OFF CACHE BOOL "" FORCE)
set(GGML_AVX512_VBMI OFF CACHE BOOL "" FORCE)
set(GGML_AVX512_VNNI OFF CACHE BOOL "" FORCE)
set(GGML_AVX512_BF16 OFF CACHE BOOL "" FORCE)
set(GGML_AMX_TILE    OFF CACHE BOOL "" FORCE)
set(GGML_AMX_INT8    OFF CACHE BOOL "" FORCE)
set(GGML_AMX_BF16    OFF CACHE BOOL "" FORCE)
