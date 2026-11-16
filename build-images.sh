#!/usr/bin/env bash
set -euo pipefail
: "${CONTAINER_REGISTRY:?set CONTAINER_REGISTRY, e.g. ghcr.io/lth-fed}"
: "${CONTAINER_TAG:?set CONTAINER_TAG to the commit SHA}"
engine="${CONTAINER_ENGINE:-podman}"
git_version="${GIT_VERSION:-$(git describe --always --dirty=-modified)}"
services=("$@")
if [ ${#services[@]} -eq 0 ]; then services=(minilith fed-auth transactions); fi
for service in "${services[@]}"; do
    case "$service" in minilith|fed-auth|transactions) ;; *) echo "Unknown service: $service" >&2; exit 1 ;; esac
    "$engine" build --target "$service" --build-arg "GIT_VERSION=$git_version" \
        --label org.opencontainers.image.source=https://github.com/lth-fed/backend \
        --label "org.opencontainers.image.revision=${GIT_REVISION:-$(git rev-parse HEAD)}" \
        --tag "$CONTAINER_REGISTRY/$service:$CONTAINER_TAG" .
done
