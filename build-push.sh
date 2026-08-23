#!/bin/env bash

if [ $(git status --porcelain | wc -l) -ne "0" ]; then
    echo Please commit you changes before building.
    exit 1
fi

read -p "Version: " version

git tag $version
git push --tags

export CONTAINER_REGISTRY=registry.esek.se/esek
export CONTAINER_TAG=${version:-produciton}

podman login registry.esek.se
./build-images.sh
# also update env in `./build-images.sh`
POSTGRES_PASSWORD=unused GRAFANA_ADMIN_PASSWORD=unused RUSTFS_ACCESS_KEY=unused RUSTFS_SECRET_KEY=unused \
    podman compose -f compose.prod.yaml push fed-auth transactions minilith
