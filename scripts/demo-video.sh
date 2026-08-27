#!/usr/bin/env bash
set -euo pipefail
# One-click demo for competition video (max 5 min walkthrough)
# Shows: AI NLU (Pidgin), gift-card rate engine, dashboard approval, wallet credit

cd "$(dirname "$0")/.."
source <(grep -E '^(INTERNAL_API_SECRET)=' .env 2>/dev/null || echo "INTERNAL_API_SECRET=demo-secret-32-chars-minimum-length")

API="http://127.0.0.1:3000"
DASH="http://127.0.0.1:5173"

echo "=== MiddleMan — 5-min Competition Demo ==="
echo

# 1. Health check
echo "1) Health check"
curl -s "$API/api/v1/admin/health" | python3 -m json.tool
echo

# 2. Clean slate for recording (optional — comment out to keep history)
echo "2) Resetting demo trades (keeping users)"
docker exec middleman-postgres-1 psql -U mm_user -d middleman_db -tAc "DELETE FROM gift_card_trades; DELETE FROM transactions; DELETE FROM processed_messages; UPDATE wallets SET balance=0; UPDATE wallets SET reserved_balance=0;" >/dev/null
echo "   DB cleaned (wallets zeroed for clean before/after)"
echo

# 3. AI NLU demo — Pidgin + English → trades
echo "3) AI NLU — sending 3 WhatsApp messages (Pidgin & English)"
echo "   - 'I wan sell \$100 Apple card' (Pidgin) -> LIQUIDATE_GIFT_CARD, Apple, 100 USD"
echo "   - 'Steam \$50 card for sale' (English) -> STEAM, 50 USD"
echo "   - 'wetin dey my balance' (Pidgin) -> CHECK_BALANCE (no trade, just reply)"

for payload in \
  '{"message_id":"vid-1-'"$(date +%s)"'","sender_jid":"2348012345678@s.whatsapp.net","text_body":"I wan sell $100 Apple card","media_url":"https://res.cloudinary.com/demo/image/upload/v1/apple100.jpg"}' \
  '{"message_id":"vid-2-'"$(date +%s)"'","sender_jid":"2348012345678@s.whatsapp.net","text_body":"Steam $50 card for sale","media_url":"https://res.cloudinary.com/demo/image/upload/v1/steam50.jpg"}' \
  '{"message_id":"vid-3-'"$(date +%s)"'","sender_jid":"2348012345678@s.whatsapp.net","text_body":"wetin dey my balance"}'
do
  curl -s -X POST "$API/api/v1/debug/ingest" -H "X-Internal-Secret: $INTERNAL_API_SECRET" -H 'Content-Type: application/json' -d "$payload" | python3 -c "import json,sys; print(json.load(sys.stdin).get('queued','queued'))"
  sleep 1
done
sleep 2
echo

# 4. Show trades created by AI
echo "4) Trades created by AI (DB)"
docker exec middleman-postgres-1 psql -U mm_user -d middleman_db -c "SELECT card_brand, claimed_usd_amount, offered_ngn_rate, final_ngn_payout, status FROM gift_card_trades ORDER BY created_at;" 2>&1 | tail -n +3 | head -10
echo

# 5. Dashboard stats
echo "5) Admin Dashboard — $DASH (open in browser)"
echo "   API stats:"
curl -s "$API/api/v1/admin/dashboard" | python3 -c "import json,sys; d=json.load(sys.stdin); print(f\"   pending={d['stats']['pendingCards']} users={d['stats']['activeUsers']} trades={len(d['trades'])}\"); [print(f\"   - {t['id']} {t['card']} {t['amount']} -> {t['calculatedNaira']} [{t['status']}]\" ) for t in d['trades'][:3]]"
echo

# 6. Approve one trade — wallet credit
PENDING_ID=$(docker exec middleman-postgres-1 psql -U mm_user -d middleman_db -tAc "SELECT id FROM gift_card_trades WHERE status='PENDING' LIMIT 1" 2>&1 | tr -d ' \r\n')
if [ -n "$PENDING_ID" ] && [ "$PENDING_ID" != "" ]; then
  echo "6) Approving trade $PENDING_ID via dashboard API"
  curl -s -X POST "$API/api/v1/admin/trades/$PENDING_ID/resolve" -H 'Content-Type: application/json' -d '{"status":"Approved"}' | python3 -m json.tool
  sleep 1
  echo "   Wallet after approval:"
  docker exec middleman-postgres-1 psql -U mm_user -d middleman_db -tAc "SELECT u.whatsapp_number, w.balance FROM wallets w JOIN users u ON w.user_id=u.id WHERE w.balance>0 ORDER BY w.balance DESC" 2>&1 | head -5
else
  echo "6) No pending trade to approve"
fi
echo

# 7. Show final dashboard and wallet
echo "7) Final state — refresh dashboard at $DASH to see Approved + wallet update"
echo "   Video tip: Record browser at $DASH (Approve & Pay button) + terminal with above curls"
echo
echo "=== Demo complete — ready to record (max 5 min) ==="
echo "Slides: docs/competition/slides-outline.md"
echo "Repo: https://github.com/garba-the-analyst/MiddleMan-Terminal"
