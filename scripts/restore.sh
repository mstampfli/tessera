#!/usr/bin/env bash
# Restore the production stack from a backup directory produced by backup.sh.
# The database dump was taken with --clean --if-exists, so it drops and recreates
# objects. Usage: scripts/restore.sh <backup-dir>
set -euo pipefail

COMPOSE="docker compose -f docker-compose.prod.yml"
SRC="${1:?usage: restore.sh <backup-dir>}"
ABS_SRC="$(realpath "$SRC")"

echo "restoring database from $SRC/db.sql.gz"
gunzip -c "$SRC/db.sql.gz" | $COMPOSE exec -T db psql -U tessera -d tessera

echo "restoring content store from $SRC/cas.tar.gz"
docker run --rm -v ${COMPOSE_PROJECT_NAME:-tessera}_cas:/cas -v "$ABS_SRC":/backup alpine \
    sh -c "rm -rf /cas/* && tar xzf /backup/cas.tar.gz -C /cas"

echo "restore complete. Restart the stack: $COMPOSE up -d"
