#!/usr/bin/env bash
# Runs ON the runner VM. Push the app image using the VM's own credentials
# (short-lived token; no keys stored).
set -euo pipefail
IMAGE=$1
REGION_HOST=$(echo "$IMAGE" | cut -d/ -f1)

# Isolated docker config — see vm-base.sh.
export DOCKER_CONFIG=$(mktemp -d)
trap 'rm -rf "$DOCKER_CONFIG"' EXIT

TOKEN=$(gcloud auth print-access-token 2>/dev/null)
echo "$TOKEN" | docker login --password-stdin -u oauth2accesstoken "$REGION_HOST" >/dev/null
docker push "$IMAGE"
