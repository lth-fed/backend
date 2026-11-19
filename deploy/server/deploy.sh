#!/usr/bin/env bash
# Deploys an image tag with rolling restarts.
#
# Usage: deploy.sh <tag>
# Reads REGISTRY from /etc/teknologappen/deploy.env.
set -euo pipefail

tag="${1:?usage: deploy.sh <tag>}"
# shellcheck source=/dev/null
. /etc/teknologappen/deploy.env
: "${REGISTRY:?set REGISTRY in /etc/teknologappen/deploy.env}"

for service in fed-auth transactions minilith; do
    podman pull -q "$REGISTRY/$service:$tag" > /dev/null
    podman tag "$REGISTRY/$service:$tag" "localhost/$service:current"
done

if ! systemctl is-active -q teknologappen.target; then
    systemctl enable --now teknologappen.target
    echo "$tag" > /etc/teknologappen/deployed-tag
    exit 0
fi

wait_until_healthy() {
    local ip
    for _ in $(seq 60); do
        ip="$(podman inspect -f '{{(index .NetworkSettings.Networks "teknologappen").IPAddress}}' "$1" 2> /dev/null || true)"
        if [[ -n "$ip" ]] && curl -sfk --max-time 2 "https://$ip:8050/v0/healthcheck" > /dev/null; then
            return 0
        fi
        sleep 2
    done
    echo "$1 never became healthy" >&2
    return 1
}

# Brief downtime; clients retry.
systemctl restart fed-auth.service transactions.service

# One instance at a time keeps minilith up.
for unit in $(systemctl list-units --plain --no-legend 'minilith@*.service' | awk '{print $1}'); do
    instance="${unit#minilith@}"
    systemctl restart "$unit"
    wait_until_healthy "minilith-${instance%.service}"
done

echo "$tag" > /etc/teknologappen/deployed-tag
