# Multi-GPU Selection (v0.5.1)

Tests GPU selection on systems with multiple GPUs (e.g., integrated + discrete):

```bash
# Check detected GPUs
voxtype setup gpu
# Expected: lists all detected GPUs with vendor names

# Test GPU selection via environment variable
VOXTYPE_VULKAN_DEVICE=amd voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: AMD (via VOXTYPE_VULKAN_DEVICE)"

VOXTYPE_VULKAN_DEVICE=nvidia voxtype setup gpu | grep -A1 "GPU selection"
VOXTYPE_VULKAN_DEVICE=intel voxtype setup gpu | grep -A1 "GPU selection"
# Expected, for a vendor that is installed: "GPU selection: <Vendor> (via VOXTYPE_VULKAN_DEVICE)"
# For one that isn't: "GPU selection: auto (first available)" followed by
#   "VOXTYPE_VULKAN_DEVICE asks for <Vendor>, but no <Vendor> GPU was detected"

# Test with the Vulkan backend (packaged installs)
sudo voxtype setup gpu --enable

# The daemon reads VOXTYPE_VULKAN_DEVICE from its own environment, so set it
# on the service, not on the `record` command:
systemctl --user edit voxtype    # add: [Service]  Environment=VOXTYPE_VULKAN_DEVICE=amd
systemctl --user restart voxtype
voxtype record start
sleep 2
voxtype record stop

# Check logs for GPU selection
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "GPU selection"
```

