#!/usr/bin/env bash
# Runs ON the runner VM. Verify the base image exists in the registry;
# build and push it from the VM if missing.
set -euo pipefail
BASE_IMAGE=$1
REGION_HOST=$(echo "$BASE_IMAGE" | cut -d/ -f1)

# Isolated docker config: workflow runs leave a credential helper + expired
# WIF creds in ~/.docker/config.json; a fresh config sidesteps both.
export DOCKER_CONFIG=$(mktemp -d)
trap 'rm -rf "$DOCKER_CONFIG"' EXIT

if docker pull "$BASE_IMAGE" >/dev/null 2>&1; then
    echo "==> base image present: $BASE_IMAGE"
    exit 0
fi

echo "==> base image missing; building from Dockerfile.base"
TOKEN=$(gcloud auth print-access-token 2>/dev/null)
echo "$TOKEN" | docker login --password-stdin -u oauth2accesstoken "$REGION_HOST" >/dev/null
docker build -f Dockerfile.base -t "$BASE_IMAGE" .
docker push "$BASE_IMAGE"
