# Snapshot/Restore Memory Benchmark

dd 1 GiB to `/dev/shm` after full snapshot/restore. 2 vCPUs, 2048 MiB RAM.

Scenarios: `normal_no_uffd`, `normal_no_uffd_dirty_tracking`, `normal_uffd`, `hugepages_uffd`.

## Prerequisites

- Linux with `/dev/kvm` (bare-metal or nested virt)
- Docker
- AWS CLI v2

```bash
# Docker
sudo apt-get update && sudo apt-get install -y docker.io
sudo systemctl enable --now docker

# AWS CLI v2
curl -sL https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip -o /tmp/awscliv2.zip
cd /tmp && unzip -qo awscliv2.zip && sudo ./aws/install
rm -rf /tmp/aws /tmp/awscliv2.zip

# Hugepages (needed for hugepages_uffd scenario, 1024 pages + buffer)
echo 1200 | sudo tee /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

## Run

```bash
sudo ./tools/devtool -y test --performance -- \
    -m nonci -s \
    integration_tests/performance/test_sysbench_pause_resume.py
```

`sudo` required for the userfaultfd handler. `-m nonci` selects the benchmark. `-s` shows output.
