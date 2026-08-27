#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source <(grep -E '^(INTERNAL_API_SECRET)=' .env 2>/dev/null || echo "INTERNAL_API_SECRET=demo")

echo "Seeding 3 demo gift-card trades via debug ingest..."
for i in 1 2 3; do
  case $i in
    1) TEXT='I wan sell $100 Apple card'; BRAND='APPLE' ;;
    2) TEXT='Steam $50 card for sale'; BRAND='STEAM' ;;
    3) TEXT='Amazon $25 gift card'; BRAND='AMAZON' ;;
  esac
  ID="demo-seed-$i-$(date +%s)"
  JID="234801000000$i@s.whatsapp.net"
  IMG="https://res.cloudinary.com/demo/image/upload/v1/demo/card$i.jpg"
  echo "  -> $BRAND $TEXT"
  curl -s -X POST http://127.0.0.1:3000/api/v1/debug/ingest \
    -H "X-Internal-Secret: $INTERNAL_API_SECRET" \
    -H 'Content-Type: application/json' \
    -d "{\"message_id\":\"$ID\",\"sender_jid\":\"$JID\",\"text_body\":\"$TEXT\",\"media_url\":\"$IMG\"}" | cat
  sleep 0.5
done
echo "Done. Open http://127.0.0.1:5173 to see trades."
