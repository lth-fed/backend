#!/bin/env bash

read -p "Version: " version

export CONTAINER_REGISTRY=registry.esek.se/esek
export CONTAINER_TAG=${version:-produciton}

podman login registry.esek.se
./build-images.sh
# also update env in `./build-images.sh`
POSTGRES_PASSWORD=unused GRAFANA_ADMIN_PASSWORD=unused RUSTFS_ACCESS_KEY=unused RUSTFS_SECRET_KEY=unused \
    podman compose -f compose.prod.yaml push fed-auth transactions minilith
