<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import axios from 'axios'

const activeTab = ref('Gift Card Trades')

const navigation = [
  { name: 'Analytics & Bot Metrics', icon: '📊', category: 'General' },
  { name: 'Gift Card Trades', icon: '💳', category: 'Trading' },
  { name: 'Price Catalogue', icon: '🏷️', category: 'Trading' },
  { name: 'Transaction Reviews', icon: '🔁', category: 'Finance' },
  { name: 'Support & Disputes', icon: '💬', category: 'Support' },
  { name: 'Employee Management', icon: '👥', category: 'Super Admin' },
  { name: 'Audit Logs', icon: '🛡️', category: 'Super Admin' },
]

const giftcardTrades = ref<any[]>([])
const dashboardStats = ref({ activeUsers: 0, pendingCards: 0, todayVolume: '₦0' })
const priceCatalogue = ref<any[]>([])
const selectedImage = ref<string | null>(null)
const isLive = ref(false)
let pollTimer: number | null = null

const auditLogs = ref([
  { id: 'LOG-301', employee: 'Suleiman (Support L1)', action: 'Approved Giftcard GC-9078', time: '10:42 AM', status: 'Success' },
  { id: 'LOG-302', employee: 'Aisha (Risk Lead)', action: 'Updated Steam Rate to ₦1,450/$', time: '09:15 AM', status: 'Notice' },
])

const API_BASE = '' // uses Vite proxy /api -> 3000

async function fetchDashboard() {
  try {
    // Dashboard is public for demo; resolve still works without token (see crates/mm-api/src/handlers/admin.rs)
    const response = await axios.get(`${API_BASE}/api/v1/admin/dashboard`)
    giftcardTrades.value = response.data.trades || []
    dashboardStats.value = response.data.stats || { activeUsers: 0, pendingCards: 0, todayVolume: '₦0' }
    if (response.data.catalogue) priceCatalogue.value = response.data.catalogue
    isLive.value = true
  } catch (error) {
    console.error('Failed to connect to Rust API:', error)
    isLive.value = false
  }
}

onMounted(async () => {
  await fetchDashboard()
  pollTimer = window.setInterval(fetchDashboard, 3000)
})

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})

// Action handler for the buttons — handles both legacy "Approved"/"Rejected" and new "approve"/"reject"
const resolveTrade = async (trade: any, status: string) => {
  let reason = ''
  if (status === 'Rejected' || status === 'reject' || status === 'rejected') {
    const input = window.prompt("Enter rejection reason (User will see this on WhatsApp):", "Card has already been redeemed.")
    if (input === null) return
    reason = input
  }

  const originalStatus = trade.status
  trade.status = 'Processing...'

  try {
    // Backend now accepts both {status} and {action} (case-insensitive) — see handlers/admin.rs
    await axios.post(`${API_BASE}/api/v1/admin/trades/${trade.db_id}/resolve`, {
      status,
      action: status.toLowerCase(),
      reason
    })
    // Optimistic update; will be corrected on next poll
    trade.status = status === 'Approved' || status === 'approve' ? 'Approved' : 'Rejected'
    setTimeout(fetchDashboard, 800)
  } catch (error: any) {
    console.error('Failed to resolve trade:', error)
    const msg = error?.response?.data?.error || 'Failed to execute resolution. Ensure Rust server is running on :3000.'
    alert(msg)
    trade.status = originalStatus
  }
}
</script>

<template>
  <div class="min-h-screen bg-obsidian text-silver-light flex font-sans relative">
    
    <!-- Sidebar -->
    <aside class="w-72 bg-obsidian-dark border-r border-obsidian-border flex flex-col justify-between">
      <div>
        <div class="p-6 border-b border-obsidian-border bg-navy-dark">
          <div class="flex items-center space-x-3">
            <div class="w-10 h-10 rounded-lg bg-gradient-to-br from-navy-accent to-silver-metallic flex items-center justify-center font-bold text-obsidian text-xl shadow-lg">
              M
            </div>
            <div>
              <h1 class="text-lg font-bold text-silver-light tracking-wide">MiddleMan</h1>
              <p class="text-xs text-silver-muted">Enterprise Control Center</p>
            </div>
          </div>
        </div>

        <nav class="p-4 space-y-1">
          <template v-for="item in navigation" :key="item.name">
            <button
              @click="activeTab = item.name"
              :class="[
                'w-full flex items-center space-x-3 px-4 py-3 rounded-lg text-sm font-medium transition-all duration-150',
                activeTab === item.name
                  ? 'bg-navy text-silver-light border-l-4 border-silver-metallic shadow-md'
                  : 'text-silver-muted hover:bg-obsidian-card hover:text-silver-light'
              ]"
            >
              <span>{{ item.icon }}</span>
              <span>{{ item.name }}</span>
            </button>
          </template>
        </nav>
      </div>
    </aside>

    <!-- Main Workspace -->
    <main class="flex-1 flex flex-col h-screen overflow-hidden bg-obsidian">
      <header class="bg-navy-dark border-b border-obsidian-border px-8 py-4 flex justify-between items-center shrink-0">
        <div class="flex items-center space-x-4">
          <h2 class="text-xl font-bold text-silver-light">{{ activeTab }}</h2>
          <span :class="[
            'px-3 py-1 rounded-full text-xs border',
            isLive ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30' : 'bg-red-500/10 text-red-400 border-red-500/30'
          ]">{{ isLive ? '● Live' : '○ Offline — check API on :3000' }}</span>
        </div>
        <button @click="fetchDashboard()" class="text-xs px-3 py-1 rounded bg-obsidian-card border border-obsidian-border text-silver-muted hover:text-silver-light">↻ Refresh</button>
      </header>

      <div class="flex-1 overflow-auto p-8 space-y-6">
        <!-- TAB 1: GIFT CARD TRADES -->
        <div v-if="activeTab === 'Gift Card Trades'" class="space-y-6">
          <div class="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
            <div class="bg-obsidian-card p-6 rounded-xl border border-obsidian-border">
              <h3 class="text-sm font-medium text-silver-dark">Pending Gift Cards</h3>
              <p class="text-3xl font-bold text-silver-light mt-2">{{ dashboardStats.pendingCards }}</p>
            </div>
            <div class="bg-obsidian-card p-6 rounded-xl border border-obsidian-border">
              <h3 class="text-sm font-medium text-silver-dark">Today's Volume</h3>
              <p class="text-3xl font-bold text-silver-light mt-2">{{ dashboardStats.todayVolume }}</p>
            </div>
            <div class="bg-obsidian-card p-6 rounded-xl border border-obsidian-border">
              <h3 class="text-sm font-medium text-silver-dark">Active Users (Live DB)</h3>
              <p class="text-3xl font-bold text-emerald-400 mt-2">{{ dashboardStats.activeUsers }}</p>
            </div>
          </div>

           <div v-if="giftcardTrades.length === 0" class="text-center py-16 border border-dashed border-obsidian-border rounded-xl bg-obsidian-card/50">
              <p class="text-silver-muted text-sm">No trades yet — send a WhatsApp gift-card photo or run <code class="bg-obsidian-dark px-2 py-1 rounded">bash scripts/demo-seed.sh</code></p>
              <p class="text-xs text-silver-dark mt-2">API must be running on :3000 and Redis on :6379</p>
            </div>
            <div v-else class="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <div v-for="trade in giftcardTrades" :key="trade.id" class="bg-obsidian-card border border-obsidian-border rounded-xl p-6 space-y-4">
              <div class="flex justify-between items-start">
                <div>
                  <span class="text-xs font-mono text-silver-dark">{{ trade.id }}</span>
                  <h4 class="text-base font-bold text-silver-light mt-1">{{ trade.card }} ({{ trade.amount }})</h4>
                  <p class="text-xs text-silver-muted">User: {{ trade.user }}</p>
                </div>
                <span :class="[
                  'px-3 py-1 rounded-full text-xs font-semibold',
                  trade.status === 'Pending Review' ? 'bg-amber-500/10 text-amber-400 border border-amber-500/30' : 
                  trade.status === 'Approved' ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/30' : 
                  trade.status === 'Processing...' ? 'bg-blue-500/10 text-blue-400 border border-blue-500/30 animate-pulse' :
                  'bg-red-500/10 text-red-400 border border-red-500/30'
                ]">
                  {{ trade.status }}
                </span>
              </div>

              <div class="p-4 bg-obsidian-dark rounded-lg border border-obsidian-border flex items-center justify-between">
                <div>
                  <p class="text-xs text-silver-muted">Payout Calculation</p>
                  <p class="text-lg font-bold text-emerald-400">{{ trade.calculatedNaira }}</p>
                </div>
                <button 
                  @click="trade.image_url ? selectedImage = trade.image_url : null"
                  :disabled="!trade.image_url"
                  :class="[
                    'px-4 py-2 text-xs font-semibold rounded-lg transition-colors border',
                    trade.image_url ? 'bg-navy hover:bg-navy-accent text-silver-light border-silver-dark/30 shadow-md' : 'bg-obsidian text-silver-dark border-obsidian-border cursor-not-allowed'
                  ]"
                >
                  {{ trade.image_url ? 'Inspect Image' : 'No Image' }}
                </button>
              </div>

              <!-- Dynamic Approval Buttons -->
              <div class="flex space-x-3 pt-2">
                <button 
                  @click="resolveTrade(trade, 'Approved')"
                  :disabled="trade.status === 'Approved' || trade.status === 'Rejected'"
                  class="flex-1 py-2.5 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-30 disabled:cursor-not-allowed text-white font-semibold text-xs rounded-lg transition-colors shadow-md">
                  Approve & Pay
                </button>
                <button 
                  @click="resolveTrade(trade, 'Rejected')"
                  :disabled="trade.status === 'Approved' || trade.status === 'Rejected'"
                  class="flex-1 py-2.5 bg-red-600/80 hover:bg-red-500 disabled:opacity-30 disabled:cursor-not-allowed text-white font-semibold text-xs rounded-lg transition-colors">
                  Reject / Flag
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- TAB 2: PRICE CATALOGUE -->
        <div v-if="activeTab === 'Price Catalogue'" class="space-y-6">
          <div class="bg-obsidian-card border border-obsidian-border rounded-xl overflow-hidden">
            <table class="w-full text-left border-collapse">
              <thead class="bg-navy-dark text-silver-muted text-xs uppercase">
                <tr><th class="p-4">Brand</th><th class="p-4">Rate (₦ / $)</th><th class="p-4">Status</th></tr>
              </thead>
              <tbody class="divide-y divide-obsidian-border text-sm">
                <tr v-for="item in priceCatalogue" :key="item.id" class="hover:bg-obsidian-dark/50">
                  <td class="p-4 font-semibold text-silver-light">{{ item.brand }}</td>
                  <td class="p-4 font-mono text-emerald-400 font-bold">₦{{ item.ratePerDollar }}</td>
                  <td class="p-4"><span class="text-xs font-bold" :class="item.status === 'Active' ? 'text-emerald-400' : 'text-red-400'">● {{ item.status }}</span></td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

      </div>
    </main>

    <!-- Image Modal -->
    <div v-if="selectedImage" class="absolute inset-0 z-50 flex items-center justify-center bg-obsidian-dark/80 backdrop-blur-sm p-6">
      <div class="bg-obsidian-card border border-obsidian-border rounded-xl shadow-2xl max-w-2xl w-full flex flex-col overflow-hidden">
        <div class="p-4 bg-navy-dark border-b border-obsidian-border flex justify-between items-center">
          <h3 class="text-lg font-bold text-silver-light tracking-wide">Media Inspection</h3>
          <button @click="selectedImage = null" class="text-silver-muted hover:text-red-400 transition-colors">X</button>
        </div>
        <div class="p-6 bg-obsidian flex justify-center items-center min-h-[400px]">
          <img :src="selectedImage" alt="User uploaded gift card" class="max-w-full max-h-[60vh] object-contain rounded-lg border border-obsidian-border shadow-md" />
        </div>
      </div>
    </div>

  </div>
</template>