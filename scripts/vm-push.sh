#!/usr/bin/env bash
# Runs ON the runner VM. Push the app image using the VM's own credentials
# (short-lived token; no keys stored).
set -euo pipefail
IMAGE=$1
REGION_HOST=$(echo "$IMAGE" | cut -d/ -f1)

TOKEN=$(gcloud auth print-access-token 2>/dev/null)
echo "$TOKEN" | docker login --password-stdin -u oauth2accesstoken "$REGION_HOST" >/dev/null
docker push "$IMAGE"
docker logout "$REGION_HOST" >/dev/null
