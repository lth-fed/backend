#!/usr/bin/env bash
# Backs up Postgres and S3 with restic.
set -euo pipefail

: "${RESTIC_REPOSITORY:?set RESTIC_REPOSITORY in /etc/teknologappen/backup.env}"

restic cat config > /dev/null 2>&1 || restic init

# Dump to a file first: no partial snapshots.
dump_dir=/var/backups/teknologappen
install -d -m 700 "$dump_dir"
podman exec fed-postgres sh -c 'pg_dumpall -U "$POSTGRES_USER"' > "$dump_dir/postgres.sql"
restic backup --tag postgres "$dump_dir/postgres.sql"

restic backup --tag s3 "${S3_DATA_DIR:-/fed/s3}"

restic forget --prune --keep-daily 7 --keep-weekly 4 --keep-monthly 6
