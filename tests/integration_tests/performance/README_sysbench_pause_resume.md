# Snapshot/Restore Memory Benchmark

Writes 1 GiB to `/dev/shm` (via `dd`) inside a guest that was just
restored from a full snapshot. Reports wall-clock duration and
throughput.

Four scenarios:
- **normal_no_uffd** — regular 4 KiB pages, kernel page faults
- **normal_no_uffd_dirty_tracking** — regular pages, dirty tracking enabled
- **normal_uffd** — regular pages, userfaultfd on-demand paging
- **hugepages_uffd** — 2 MiB hugepages, userfaultfd on-demand paging

## Prerequisites

- Linux host with `/dev/kvm` (bare-metal or nested virt).
- Docker running.
- AWS CLI v2 (artifacts are on a public S3 bucket, no credentials
  needed).

### Hugepages

The hugepages scenario needs 2 MiB hugepages pre-allocated on the host.
The VM uses 2048 MiB RAM, so at least 1024 pages (+ some buffer):

```bash
echo 1200 | sudo tee /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

## Running

```bash
sudo ./tools/devtool -y test --performance -- \
    -m nonci -s \
    integration_tests/performance/test_sysbench_pause_resume.py
```

`sudo` is required — the userfaultfd handler needs permissions to open
memory files. This builds Firecracker, downloads artifacts, and runs
the test. `-m nonci` selects the benchmark (excluded by default).
`-s` shows the output.
