# GPU Backend Switching

Test transitions between CPU and GPU backends (engine-aware):

```bash
# Check current status
voxtype setup gpu

# Whisper mode (symlink points to voxtype-vulkan or voxtype-avx*)
# --enable switches to Vulkan, --disable switches to best CPU
ls -la /usr/bin/voxtype  # Verify current symlink
sudo voxtype setup gpu --enable   # Switch to Vulkan
ls -la /usr/bin/voxtype  # Should point to voxtype-vulkan
sudo voxtype setup gpu --disable  # Switch to best CPU (avx512 or avx2)
ls -la /usr/bin/voxtype  # Should point to voxtype-avx512 or voxtype-avx2

# ONNX mode (symlink points to voxtype-onnx-*)
# --enable switches to CUDA, --disable switches to best ONNX CPU
sudo ln -sf /usr/lib/voxtype/voxtype-onnx-avx512 /usr/bin/voxtype
sudo voxtype setup gpu --enable   # Switch to ONNX CUDA
ls -la /usr/bin/voxtype  # Should point to voxtype-onnx-cuda
sudo voxtype setup gpu --disable  # Switch to best ONNX CPU
ls -la /usr/bin/voxtype  # Should point to voxtype-onnx-avx512

# Restore to Whisper Vulkan for normal use
sudo ln -sf /usr/lib/voxtype/voxtype-vulkan /usr/bin/voxtype
```

