# Quick Smoke Test Script

```bash
#!/bin/bash
# quick-smoke-test.sh - Run after new build install

set -e
echo "=== Voxtype Smoke Tests ==="

echo -n "Version: "
voxtype --version

echo -n "Status: "
voxtype status

echo "Recording 3 seconds..."
voxtype record start
sleep 3
voxtype record stop
echo "Done."

echo ""
echo "Check logs:"
journalctl --user -u voxtype --since "30 seconds ago" --no-pager | tail -10
```

