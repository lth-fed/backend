#!/usr/bin/env bash
# Configures the host. Safe to rerun.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
config=/etc/teknologappen

# Security updates, reboot at night.
cat > /etc/apt/apt.conf.d/52teknologappen <<'CONF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-Time "04:30";
CONF

install -d -m 700 "$config" "$config/secrets"
install -d /fed/postgres /fed/s3
if [[ ! -f "$config/restic-password" ]]; then
    # Store a copy in the password manager.
    head -c 32 /dev/urandom | base64 > "$config/restic-password"
    chmod 600 "$config/restic-password"
fi
if [[ ! -f "$config/backup.env" ]]; then
    cat > "$config/backup.env" <<'CONF'
# Hetzner Storage Box, e.g. sftp:u123@u123.your-storagebox.de:/teknologappen
RESTIC_REPOSITORY=
RESTIC_PASSWORD_FILE=/etc/teknologappen/restic-password
S3_DATA_DIR=/fed/s3
CONF
    chmod 600 "$config/backup.env"
fi

# Quadlets become systemd units on daemon-reload.
rm -rf /etc/containers/systemd/teknologappen
install -d /etc/containers/systemd/teknologappen
install -m 644 "$here"/quadlets/* /etc/containers/systemd/teknologappen/
install -m 644 "$here"/systemd/* /etc/systemd/system/
systemctl daemon-reload

systemctl enable --now podman.socket teknologappen-backup.timer
systemctl start traefik.service

# Avoids restart loops before secrets exist.
if [[ -f "$config/postgres.env" ]]; then
    systemctl enable teknologappen.target
else
    echo "Add secrets to $config, then run deploy.sh."
    echo "Write env values without quotes."
fi
