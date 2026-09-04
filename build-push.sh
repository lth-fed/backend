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

# Mirror the release images to Docker Hub. `docker.io` is the registry
# endpoint used for repositories hosted at hub.docker.com.
podman login docker.io
for service in fed-auth transactions minilith; do
    podman tag \
        "${CONTAINER_REGISTRY}/${service}:${CONTAINER_TAG}" \
        "docker.io/esekmacapar/${service}:${CONTAINER_TAG}"
    podman push "docker.io/esekmacapar/${service}:${CONTAINER_TAG}"
    podman tag \
        "${CONTAINER_REGISTRY}/${service}:${CONTAINER_TAG}" \
        "docker.io/esekmacapar/${service}:latest"
    podman push "docker.io/esekmacapar/${service}:latest"
done
