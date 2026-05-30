# Multi-GPU Selection (v0.5.1)

Tests GPU selection on systems with multiple GPUs (e.g., integrated + discrete):

```bash
# Check detected GPUs
voxtype setup gpu
# Expected: lists all detected GPUs with vendor names

# Test GPU selection via environment variable
VOXTYPE_VULKAN_DEVICE=amd voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: AMD (via VOXTYPE_VULKAN_DEVICE)"

VOXTYPE_VULKAN_DEVICE=nvidia voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: NVIDIA (via VOXTYPE_VULKAN_DEVICE)"

VOXTYPE_VULKAN_DEVICE=intel voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: Intel (via VOXTYPE_VULKAN_DEVICE)"

# Test with Vulkan binary
sudo ln -sf /usr/lib/voxtype/voxtype-vulkan /usr/local/bin/voxtype
systemctl --user restart voxtype

# Record with specific GPU selected
VOXTYPE_VULKAN_DEVICE=amd voxtype record start
sleep 2
voxtype record stop

# Check logs for GPU selection
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "GPU selection"
```

