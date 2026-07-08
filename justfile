# Developer task runner for tessera. Run `just` to list targets.
# On this host the shell DOCKER_HOST points at a dead podman socket; these
# targets pin the real Docker daemon.

export DOCKER_HOST := "unix:///var/run/docker.sock"
export DATABASE_URL := env_var_or_default("DATABASE_URL", "postgres://tessera:tessera@127.0.0.1:5432/tessera")

default:
    @just --list

# Bring up the dev Postgres (pgvector).
db-up:
    docker compose up -d
    @echo "waiting for postgres..."
    @until docker compose exec -T db pg_isready -U tessera -d tessera >/dev/null 2>&1; do sleep 1; done
    @echo "postgres ready"

# Tear down the dev Postgres (keeps the volume).
db-down:
    docker compose down

# Apply migrations.
migrate:
    cargo run --bin tesserad -- migrate

# Run the server.
serve:
    cargo run --bin tesserad -- serve

# The full local gate mirroring CI.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Format the whole workspace.
fmt:
    cargo fmt --all
