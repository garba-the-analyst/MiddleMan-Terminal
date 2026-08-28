#!/usr/bin/env bash
set -euo pipefail
# Case Study 1 demo: AI Customer Support Assistant (NexaConnect)
# Covers: FAQ, classification, sentiment/urgency, escalation, KB, analytics, RBAC, catalogue edit

BASE="http://localhost:3000"
SECRET=$(grep INTERNAL_API_SECRET .env | cut -d= -f2 | tr -d '"\r' | tail -n1)
if [ -z "$SECRET" ]; then SECRET="45287219ee22e3ab05877ed6f927008c95fde0df9b62d01ccf44f5e4a53d286e"; fi

echo "=== MiddleMan — Case Study 1: AI Customer Support Assistant Demo ==="
echo ""
echo "1) Login as super_admin (garbaabdullahi344@gmail.com / Babawo_344)"
TOKEN=$(curl -s -X POST $BASE/api/v1/admin/login -H "Content-Type: application/json" -d '{"email":"garbaabdullahi344@gmail.com","password":"Babawo_344"}' | jq -r .token)
echo "   token ${TOKEN:0:12}... role SUPER_ADMIN"

echo ""
echo "2) Bot analytics — DB tables: bot_interactions(120+), bot_analytics(14d), knowledge_base(10), gift_card_trades"
curl -s $BASE/api/v1/admin/bot/stats -H "x-admin-token: $TOKEN" | jq '{total: .total_interactions, escalated: .escalated_count, escalation_rate, today: .today_interactions, categories: .by_category}'

echo ""
echo "3) Knowledge base retrieval — FAQ"
curl -s "$BASE/api/v1/admin/kb?q=refund" | jq '.[].question'

echo ""
echo "4) Simulate 3 customer enquiries (classification + sentiment + urgency + escalation + KB)"
for msg in "What is your refund policy?" "My card pending since morning URGENT!!! I need refund now this is terrible" "I wan sell \$100 Steam card"; do
  MID="cs1-$(date +%s%N)-$RANDOM"
  echo "   -> \"$msg\""
  curl -s -X POST $BASE/api/v1/debug/ingest -H "Content-Type: application/json" -H "X-Internal-Secret: $SECRET" -d "{\"message_id\":\"$MID\",\"sender_jid\":\"2348012345999@s.whatsapp.net\",\"text_body\":\"$msg\"}" | jq .
  sleep 1
done
sleep 1
echo ""
echo "   Bot interactions (last 3):"
docker exec middleman-postgres-1 psql -U mm_user -d middleman_db -c "SELECT left(inbound_text,40) as text, category, sentiment, urgency, escalated FROM bot_interactions ORDER BY created_at DESC LIMIT 3"

echo ""
echo "5) Price catalogue — editable (RBAC: SUPER_ADMIN & OPERATIONS_MANAGER can edit)"
echo "   Before: SEPHORA"
curl -s $BASE/api/v1/admin/dashboard -H "x-admin-token: $TOKEN" | jq '.catalogue[] | select(.brand=="SEPHORA")'
echo "   Updating SEPHORA 1550 -> 1600"
CID=$(curl -s $BASE/api/v1/admin/dashboard -H "x-admin-token: $TOKEN" | jq '.catalogue[] | select(.brand=="SEPHORA") | .id')
curl -s -X POST $BASE/api/v1/admin/catalogue/$CID -H "Content-Type: application/json" -H "x-admin-token: $TOKEN" -d '{"brand":"SEPHORA","country":"US","card_format":"ECODE","rate_per_dollar":1600}' | jq .
echo "   Revert to 1550 for clean state"
curl -s -X POST $BASE/api/v1/admin/catalogue/$CID -H "Content-Type: application/json" -H "x-admin-token: $TOKEN" -d '{"brand":"SEPHORA","country":"US","card_format":"ECODE","rate_per_dollar":1550}' | jq . >/dev/null

echo ""
echo "6) Employee management — RBAC (only SUPER_ADMIN can create)"
echo "   Employees:"
curl -s $BASE/api/v1/admin/employees -H "x-admin-token: $TOKEN" | jq 'map({email, role})'
echo "   Support agent trying to create employee (should be forbidden):"
STOKEN=$(curl -s -X POST $BASE/api/v1/admin/login -H "Content-Type: application/json" -d '{"email":"support@middleman.com","password":"Support123!"}' | jq -r .token)
curl -s -X POST $BASE/api/v1/admin/employees -H "Content-Type: application/json" -H "x-admin-token: $STOKEN" -d '{"email":"fail@test.com","password":"Fail123!","role":"SUPPORT_AGENT"}' | jq .

echo ""
echo "7) Bot inbox — escalated interactions (human handoff)"
curl -s "$BASE/api/v1/admin/bot/interactions?escalated_only=true&limit=2" -H "x-admin-token: $TOKEN" | jq '.[] | {text: .inbound_text, category, sentiment, urgency, escalated}'

echo ""
echo "=== Demo ready ==="
echo "Dashboard: http://localhost:5173  (login garbaabdullahi344@gmail.com / Babawo_344)"
echo "API: $BASE/api/v1/admin/health"
echo "DB: bot_interactions, knowledge_base, bot_analytics, price_catalogue_audit, admin_employees (RBAC), gift_card_trades"
