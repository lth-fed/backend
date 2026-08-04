#!/usr/bin/env bash
set -euo pipefail

: "${CONTAINER_REGISTRY:?set CONTAINER_REGISTRY, for example registry.esek.se/esek}"
: "${CONTAINER_TAG:?set CONTAINER_TAG to an immutable release tag}"

git_version="${GIT_VERSION:-$(git describe --always --dirty=-modified)}"

POSTGRES_PASSWORD=unused GRAFANA_ADMIN_PASSWORD=unused \
  RUSTFS_ACCESS_KEY=unused RUSTFS_SECRET_KEY=unused \
  podman compose -f compose.prod.yaml build \
    --build-arg "GIT_VERSION=$git_version" "$@"
