# Parakeet Backend Switching

Test switching between Whisper and Parakeet engines:

```bash
# Check current status
voxtype setup parakeet

# Enable Parakeet (switches symlink to best parakeet binary)
sudo voxtype setup parakeet --enable
ls -la /usr/bin/voxtype  # Should point to voxtype-onnx-cuda or voxtype-onnx-avx*

# Disable Parakeet (switches back to equivalent Whisper binary)
sudo voxtype setup parakeet --disable
ls -la /usr/bin/voxtype  # Should point to voxtype-vulkan or voxtype-avx*

# Verify systemd service was updated
grep ExecStart ~/.config/systemd/user/voxtype.service
```

