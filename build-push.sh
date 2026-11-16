#!/usr/bin/env bash
set -euo pipefail
# Authenticate with the registry before running; this same entrypoint is used by CI.
export CONTAINER_REGISTRY="${CONTAINER_REGISTRY:-ghcr.io/lth-fed}"
export CONTAINER_TAG="${CONTAINER_TAG:-$(git rev-parse HEAD)}"
engine="${CONTAINER_ENGINE:-podman}"
./build-images.sh "$@"
services=("$@")
if [ ${#services[@]} -eq 0 ]; then services=(minilith fed-auth transactions); fi
for service in "${services[@]}"; do
    "$engine" push "$CONTAINER_REGISTRY/$service:$CONTAINER_TAG"
    # Backend configuration is runtime-only: both environments use the same build.
    for environment in staging production; do
        "$engine" tag "$CONTAINER_REGISTRY/$service:$CONTAINER_TAG" "$CONTAINER_REGISTRY/$service:$environment-$CONTAINER_TAG"
        "$engine" push "$CONTAINER_REGISTRY/$service:$environment-$CONTAINER_TAG"
    done
done
