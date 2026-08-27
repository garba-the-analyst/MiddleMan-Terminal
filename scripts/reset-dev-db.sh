#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "Stopping stack and deleting dev volumes (pgdata, redis)..."
docker compose down -v --remove-orphans >/dev/null 2>&1 || true

echo "Starting postgres + redis..."
docker compose up -d postgres redis

echo "Waiting for postgres..."
until docker compose exec -T postgres pg_isready -U mm_user -d middleman_db >/dev/null 2>&1; do
  sleep 1
done

bash scripts/migrate.sh
echo "Dev database reset complete (localhost:5434/middleman_db)."
