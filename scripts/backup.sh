#!/usr/bin/env bash
# Back up the production stack: the Postgres database and the content store.
# Usage: scripts/backup.sh [destination-dir]
set -euo pipefail

COMPOSE="docker compose -f docker-compose.prod.yml"
DEST="${1:-./backups/$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$DEST"
ABS_DEST="$(realpath "$DEST")"

echo "backing up database -> $DEST/db.sql.gz"
$COMPOSE exec -T db pg_dump --clean --if-exists -U tessera -d tessera | gzip >"$DEST/db.sql.gz"

echo "backing up content store -> $DEST/cas.tar.gz"
docker run --rm -v tessera_cas:/cas -v "$ABS_DEST":/backup alpine \
    tar czf /backup/cas.tar.gz -C /cas .

echo "backup complete: $DEST"
