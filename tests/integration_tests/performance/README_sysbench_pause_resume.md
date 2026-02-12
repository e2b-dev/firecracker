# Snapshot/Restore Memory Benchmark

Writes 1 GiB to `/dev/shm` (via `dd`) inside a guest that was just
restored from a full snapshot. Reports wall-clock duration and
throughput.

## Prerequisites

- Linux host with `/dev/kvm` (bare-metal or nested virt).
- Docker running.
- AWS CLI v2 (artifacts are on a public S3 bucket, no credentials
  needed).

## Running

```bash
./tools/devtool -y test --performance -- \
    -m nonci -s \
    integration_tests/performance/test_sysbench_pause_resume.py
```

This builds Firecracker, downloads artifacts, and runs the test.
`-m nonci` is required because `pytest.ini` excludes that marker by
default. `-s` shows the benchmark output.
