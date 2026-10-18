#!/usr/bin/env bash
# Runs ON the runner VM. Build the thin app image (native amd64; BuildKit
# cache mounts persist on the VM's data disk between builds).
set -euo pipefail
BASE_IMAGE=$1
IMAGE=$2

docker build -f Dockerfile.app --build-arg BASE_IMAGE="$BASE_IMAGE" -t "$IMAGE" .
