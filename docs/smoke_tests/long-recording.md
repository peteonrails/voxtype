# Long Recording

Test recording near the max_duration_secs limit:

```bash
# Check current max duration
voxtype config | grep max_duration

# Start a long recording (default max is 60s)
# The daemon should auto-stop at the limit
voxtype record start
echo "Recording... will auto-stop at max_duration_secs"
# Wait or manually stop before limit:
sleep 10
voxtype record stop

# To test auto-cutoff, set max_duration_secs = 5 in config and record longer
```

