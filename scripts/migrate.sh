#!/usr/bin/env bash
set -euo pipefail

DB_CONTAINER="${DB_CONTAINER:-middleman-postgres-1}"
DB_USER="${DB_USER:-mm_user}"
DB_NAME="${DB_NAME:-middleman_db}"

cd "$(dirname "$0")/.."

for file in migrations/*.sql; do
  echo "Applying $file"
  docker exec -i "$DB_CONTAINER" psql -v ON_ERROR_STOP=1 -U "$DB_USER" -d "$DB_NAME" < "$file"
done

echo "Migrations applied."
