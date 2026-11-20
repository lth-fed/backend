#!/usr/bin/env bash
# Tests cloud-init.yaml in a local QEMU VM.
#
# Usage: ./test-vm.sh [stop]
# Needs: qemu-system-x86_64, qemu-img, genisoimage, python3, KVM.
# Uses the working tree, including uncommitted files.
# Pulls the public docker.io/esekmacapar images.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$here" rev-parse --show-toplevel)"
work="${WORK_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/teknologappen-vm}"
image_url=https://cloud.debian.org/images/cloud/trixie/latest
image=debian-13-genericcloud-amd64.qcow2
ssh_port=2222
https_port=8443
http_port="${HTTP_PORT:-18765}"
vm_memory="${VM_MEMORY:-2048}"

mkdir -p "$work"

ssh_vm() {
    ssh -q -i "$work/id_ed25519" -p "$ssh_port" \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=5 admin@127.0.0.1 "$@"
}

stop_vm() {
    if [[ -f "$work/qemu.pid" ]]; then
        kill "$(cat "$work/qemu.pid")" 2> /dev/null || true
        rm -f "$work/qemu.pid"
    fi
}

if [[ "${1:-}" == stop ]]; then
    stop_vm
    exit 0
fi

fetch_image() {
    if [[ -f "$work/$image" ]]; then
        return
    fi
    curl -sSfL -o "$work/$image" "$image_url/$image"
    curl -sSfL -o "$work/SHA512SUMS" "$image_url/SHA512SUMS"
    (cd "$work" && grep " $image\$" SHA512SUMS | sha512sum -c --quiet)
}

# Serves the working tree as a git repo.
serve_repo_snapshot() {
    local index="$work/index" snap="$work/repo.git" tree commit
    rm -rf "$index" "$snap"
    GIT_INDEX_FILE="$index" git -C "$repo_root" add -A
    tree="$(GIT_INDEX_FILE="$index" git -C "$repo_root" write-tree)"
    commit="$(git -C "$repo_root" commit-tree "$tree" -m "VM test snapshot")"
    git init -q --bare "$snap"
    git -C "$repo_root" push -q "$snap" "$commit:refs/heads/vm-test"
    git -C "$snap" update-server-info
    python3 -m http.server "$http_port" --bind 127.0.0.1 \
        --directory "$work" > "$work/http.log" 2>&1 &
    trap 'kill $! 2> /dev/null || true' EXIT
    sleep 1
    if ! curl -sf "http://127.0.0.1:$http_port/repo.git/HEAD" > /dev/null; then
        echo "Repo server failed; see $work/http.log" >&2
        exit 1
    fi
}

boot_vm() {
    stop_vm
    [[ -f "$work/id_ed25519" ]] || ssh-keygen -q -t ed25519 -N "" -f "$work/id_ed25519"
    local key
    key="$(cat "$work/id_ed25519.pub")"
    sed -e "s|@@ADMIN_SSH_KEY@@|$key|" -e "s|@@DEPLOY_SSH_KEY@@|$key|" \
        -e "s|@@REPO_URL@@|http://10.0.2.2:$http_port/repo.git|" \
        -e "s|@@REPO_REF@@|vm-test|" \
        "$here/cloud-init.yaml" > "$work/user-data"
    printf 'instance-id: test-%s\nlocal-hostname: teknologappen\n' "$(date +%s)" \
        > "$work/meta-data"
    genisoimage -quiet -output "$work/seed.iso" -volid cidata -joliet -rock \
        "$work/user-data" "$work/meta-data"
    rm -f "$work/disk.qcow2"
    qemu-img create -q -f qcow2 -F qcow2 -b "$work/$image" "$work/disk.qcow2" 40G
    # 2 vCPU like a CX23; less RAM.
    qemu-system-x86_64 -enable-kvm -cpu host -smp 2 -m "$vm_memory" \
        -drive "file=$work/disk.qcow2,if=virtio" -cdrom "$work/seed.iso" \
        -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$ssh_port-:22,hostfwd=tcp:127.0.0.1:$https_port-:443" \
        -display none -serial "file:$work/serial.log" \
        -daemonize -pidfile "$work/qemu.pid"
}

wait_for_ssh() {
    for _ in $(seq 120); do
        if ssh_vm true 2> /dev/null; then
            return
        fi
        sleep 5
    done
    echo "SSH never came up; see $work/serial.log" >&2
    exit 1
}

# Copies test secrets, like an admin would.
install_test_secrets() {
    ssh_vm sudo bash -s << 'EOF'
set -euo pipefail
cd /srv/teknologappen/backend
config=/etc/teknologappen
for service in minilith fed-auth transactions; do
    db="$service"
    [[ "$service" == minilith ]] && db=fed
    [[ "$service" == fed-auth ]] && db=auth
    # Podman keeps quotes as part of values.
    grep -v '^DATABASE_URL=' "$service/.env.example" \
        | sed -E 's/^([A-Z0-9_]+)="(.*)"$/\1=\2/' > "$config/$service.env"
    echo "DATABASE_URL=postgres://postgres:vm-test@fed-postgres:5432/$db" >> "$config/$service.env"
    echo DEBUG=1 >> "$config/$service.env"
done
touch "$config/secrets/firebase-service-account.json" "$config/secrets/AuthKey.p8"
printf 'POSTGRES_USER=postgres\nPOSTGRES_PASSWORD=vm-test\n' > "$config/postgres.env"
printf 'OTEL_REMOTE_ENDPOINT=http://127.0.0.1:4318\nOTEL_REMOTE_AUTHORIZATION=Bearer vm-test\n' > "$config/collector.env"
printf 'RUSTFS_ACCESS_KEY=rustfsadmin\nRUSTFS_SECRET_KEY=rustfsadmin\n' > "$config/s3.env"
echo REGISTRY=docker.io/esekmacapar > "$config/deploy.env"
cat > "$config/backup.env" << 'CONF'
RESTIC_REPOSITORY=/var/backups/restic-test
RESTIC_PASSWORD_FILE=/etc/teknologappen/restic-password
S3_DATA_DIR=/fed/s3
CONF
EOF
}

check() {
    local name="$1"
    shift
    if "$@" > /dev/null 2>&1; then
        echo "PASS $name"
    else
        echo "FAIL $name"
        failures=$((failures + 1))
    fi
}

curl_host() {
    curl -sfk --max-time 5 --resolve "$1:$https_port:127.0.0.1" \
        "https://$1:$https_port$2"
}

wait_for_route() {
    for _ in $(seq 60); do
        if curl_host "$@" > /dev/null 2>&1; then
            return 0
        fi
        sleep 5
    done
    return 1
}

check_routes() {
    check "minilith healthcheck ($1)" wait_for_route api.teknologappen.se /v0/healthcheck
    check "fed-auth certs ($1)" wait_for_route api.auth.teknologappen.se /oidc/v1/certs
    check "transactions jwks ($1)" wait_for_route transactions.teknologappen.se /v0/jwks
}

restore_has_databases() {
    local count
    count="$(ssh_vm sudo bash -c "'set -a; . /etc/teknologappen/backup.env; \
        restic dump --tag postgres latest /var/backups/teknologappen/postgres.sql'" \
        | grep -cE '^CREATE DATABASE (fed|auth|transactions) ')"
    [[ "$count" == 3 ]]
}

# Polls minilith during a deploy.
rolling_deploy_no_errors() {
    local log="$work/rolling.log" loop rc=0
    while :; do
        if curl_host api.teknologappen.se /v0/healthcheck > /dev/null 2>&1; then
            echo ok
        else
            echo error
        fi
        sleep 0.2
    done > "$log" &
    loop=$!
    ssh_vm sudo "$deploy" latest || rc=$?
    kill "$loop"
    [[ "$rc" == 0 ]] && ! grep -q error "$log"
}

wait_for_reboot() {
    ssh_vm sudo systemctl reboot || true
    sleep 15
    wait_for_ssh
}

failures=0
fetch_image
serve_repo_snapshot
boot_vm
echo "Booting VM; log in $work/serial.log"
wait_for_ssh
check "cloud-init finished cleanly" ssh_vm sudo cloud-init status --wait
check "root login disabled" ssh_vm "sudo sshd -T | grep -qx 'permitrootlogin no'"
check "password login disabled" ssh_vm "sudo sshd -T | grep -qx 'passwordauthentication no'"
check "unattended upgrades enabled" ssh_vm "apt-config dump | grep -q 'Unattended-Upgrade \"1\"'"
check "traefik running" ssh_vm systemctl is-active traefik.service
check "backup timer enabled" ssh_vm systemctl is-active teknologappen-backup.timer

check "quadlets generate units" ssh_vm \
    sudo /usr/lib/systemd/system-generators/podman-system-generator --dryrun

install_test_secrets
deploy=/srv/teknologappen/backend/deploy/server/deploy.sh
echo "Deploying (pulls images, takes minutes)"
check "first deploy" ssh_vm sudo "$deploy" latest
check_routes "first boot"
check "collector running" ssh_vm systemctl is-active fed-collector.service
check "two minilith instances" ssh_vm "sudo podman ps --format '{{.Names}}' | grep -c '^minilith-' | grep -qx 2"

check "backup ran" ssh_vm sudo systemctl start teknologappen-backup.service
check "backup holds all databases" restore_has_databases

wait_for_reboot
check "stack active after reboot" ssh_vm systemctl is-active teknologappen.target
check_routes "after reboot"

check "rolling deploy without errors" rolling_deploy_no_errors
echo "  rolling deploy: $(grep -c '^ok' "$work/rolling.log") ok, $(grep -c '^error' "$work/rolling.log") errors"

echo
echo "Memory use in the VM (${vm_memory}MB):"
ssh_vm "free -m | head -2; sudo podman stats --no-stream --format '{{.Name}} {{.MemUsage}}'" || true

echo
echo "$failures failure(s). VM still running:"
echo "  ssh -i $work/id_ed25519 -p $ssh_port admin@127.0.0.1"
echo "  $0 stop"
exit "$failures"
