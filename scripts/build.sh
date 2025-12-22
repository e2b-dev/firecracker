#!/bin/bash

set -euo pipefail

# The format will be: v<major>.<minor>.<patch>_<commit_hash> — e.g. v1.7.2_8bb88311
# Extract full version from src/firecracker/swagger/firecracker.yaml
FC_VERSION=$(awk '/^info:/{flag=1} flag && /^  version:/{print $2; exit}' src/firecracker/swagger/firecracker.yaml)
commit_hash=$(git rev-parse --short=7 HEAD)
version_name="v${FC_VERSION}_${commit_hash}"
echo "Version name: $version_name"

echo "Starting to build Firecracker version: $version_name"
tools/devtool -y build --release

mkdir -p "./build/fc/${version_name}"
cp ./build/cargo_target/x86_64-unknown-linux-musl/release/firecracker "./build/fc/${version_name}/firecracker"
echo "Finished building Firecracker version: $version_name and copied to ./build/fc/${version_name}/firecracker"
