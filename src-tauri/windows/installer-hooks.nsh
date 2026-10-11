; Older all-in-one builds installed these files directly under binaries.
; Delete only Typr's known legacy runtime files after the new CPU runtime is
; installed. The optional GPU runtime lives in app data and survives upgrades.
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\whisper-cpp-cuda.exe"
  Delete "$INSTDIR\whisper-server-cuda.exe"
  Delete "$INSTDIR\binaries\cublas64_12.dll"
  Delete "$INSTDIR\binaries\cublasLt64_12.dll"
  Delete "$INSTDIR\binaries\cudart64_12.dll"
  Delete "$INSTDIR\binaries\ggml-cuda.dll"
  Delete "$INSTDIR\binaries\ggml-base.dll"
  Delete "$INSTDIR\binaries\ggml.dll"
  Delete "$INSTDIR\binaries\whisper.dll"
  Delete "$INSTDIR\binaries\llama.dll"
  Delete "$INSTDIR\binaries\parakeet.dll"
  Delete "$INSTDIR\binaries\SDL2.dll"
  Delete "$INSTDIR\binaries\nvblas64_12.dll"
  Delete "$INSTDIR\binaries\nvrtc64_120_0.dll"
  Delete "$INSTDIR\binaries\nvrtc-builtins64_124.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-alderlake.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-cannonlake.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-cascadelake.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-haswell.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-icelake.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-sandybridge.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-skylakex.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-sse42.dll"
  Delete "$INSTDIR\binaries\ggml-cpu-x64.dll"
!macroend
