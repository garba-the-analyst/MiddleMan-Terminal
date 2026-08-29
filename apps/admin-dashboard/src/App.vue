<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import axios from 'axios'

// ===== Auth =====
const token = ref<string>(localStorage.getItem('mm_admin_token') || '')
const currentUser = ref<any>(JSON.parse(localStorage.getItem('mm_admin_user') || 'null'))
const loginEmail = ref('garbaabdullahi344@gmail.com')
const loginPassword = ref('Babawo_344')
const loginError = ref('')
const loginLoading = ref(false)
const isAuthenticated = computed(() => !!token.value)

function authHeaders() { return token.value ? { 'x-admin-token': token.value } : {} as any }

async function doLogin() {
  loginError.value = '' ; loginLoading.value = true
  try {
    const r = await axios.post('/api/v1/admin/login', { email: loginEmail.value, password: loginPassword.value })
    token.value = r.data.token
    currentUser.value = r.data.employee
    localStorage.setItem('mm_admin_token', token.value)
    localStorage.setItem('mm_admin_user', JSON.stringify(currentUser.value))
    await fetchAll()
  } catch (e:any) { loginError.value = e?.response?.data?.error || 'Login failed' }
  finally { loginLoading.value = false }
}
function logout(){ localStorage.removeItem('mm_admin_token'); localStorage.removeItem('mm_admin_user'); token.value=''; currentUser.value=null }

// ===== Navigation =====
type NavItem = { name: string, icon: string, category: string, roles?: string[] }
const allNavigation: NavItem[] = [
  { name: 'Analytics & Bot Metrics', icon: '📊', category: 'General' },
  { name: 'Bot Inbox', icon: '💬', category: 'Support' },
  { name: 'Gift Card Trades', icon: '💳', category: 'Trading' },
  { name: 'Price Catalogue', icon: '🏷️', category: 'Trading' },
  { name: 'Transactions', icon: '🔁', category: 'Finance' },
  { name: 'Fees & Charges', icon: '💰', category: 'Finance' },
  { name: 'Rates (Auto)', icon: '📈', category: 'Finance' },
  { name: 'Foreign Accounts', icon: '🌍', category: 'Finance' },
  { name: 'Knowledge Base', icon: '📚', category: 'Support' },
  { name: 'Employee Management', icon: '👥', category: 'Super Admin', roles: ['SUPER_ADMIN'] },
  { name: 'Audit Logs', icon: '🛡️', category: 'Super Admin', roles: ['SUPER_ADMIN','OPERATIONS_MANAGER'] },
]
const navigation = computed(()=> allNavigation.filter(n=> !n.roles || n.roles.includes(currentUser.value?.role || '')))
const activeTab = ref('Analytics & Bot Metrics')

// ===== Data =====
const giftcardTrades = ref<any[]>([])
const dashboardStats = ref({ activeUsers: 0, pendingCards: 0, todayVolume: '₦0' })
const priceCatalogue = ref<any[]>([])
const botStats = ref<any>(null)
const botInteractions = ref<any[]>([])
const kbItems = ref<any[]>([])
const kbSearch = ref('')
const transactions = ref<any[]>([])
const foreignAccts = ref<any[]>([])
const fees = ref<any[]>([])
const rates = ref<any[]>([])
const employees = ref<any[]>([])
const selectedImage = ref<string | null>(null)
const isLive = ref(false)
let pollTimer:number|null=null

// catalogue edit state
const showCatCreate = ref(false)
const newCat = ref({ brand: '', country: 'US', card_format: 'PHYSICAL', rate_per_dollar: 0, active: true })
const editingCat = ref<any|null>(null)

// employee create state
const showEmpCreate = ref(false)
const newEmp = ref({ email:'', password:'', full_name:'', role:'SUPPORT_AGENT' })
const roles = ref<any[]>([])

async function fetchDashboard() {
  try {
    const h = isAuthenticated.value ? { headers: authHeaders() } : undefined
    const r = await axios.get('/api/v1/admin/dashboard', h as any)
    giftcardTrades.value = r.data.trades || []
    dashboardStats.value = r.data.stats || { activeUsers:0, pendingCards:0, todayVolume:'₦0' }
    if (r.data.catalogue) priceCatalogue.value = r.data.catalogue
    isLive.value = true
  } catch { isLive.value = false }
}
async function fetchBotStats() {
  if(!isAuthenticated.value) return
  try { const r = await axios.get('/api/v1/admin/bot/stats', { headers: authHeaders() }); botStats.value = r.data } catch {}
}
async function fetchBotInteractions(escalatedOnly=false) {
  if(!isAuthenticated.value) return
  try { const r = await axios.get('/api/v1/admin/bot/interactions', { headers: authHeaders(), params:{ limit:50, escalated_only: escalatedOnly }}); botInteractions.value=r.data } catch {}
}
async function fetchEmployees() {
  if(!isAuthenticated.value) return
  try { const r = await axios.get('/api/v1/admin/employees', { headers: authHeaders() }); employees.value=r.data } catch {}
}
async function fetchTransactions(){ if(!isAuthenticated.value) return; try{ const r=await axios.get('/api/v1/admin/transactions',{headers:authHeaders()}); transactions.value=r.data }catch{} }
async function fetchForeign(){ if(!isAuthenticated.value) return; try{ const r=await axios.get('/api/v1/admin/foreign-accounts',{headers:authHeaders()}); foreignAccts.value=r.data }catch{} }
async function fetchFees(){ if(!isAuthenticated.value) return; try{ const r=await axios.get('/api/v1/admin/fees',{headers:authHeaders()}); fees.value=r.data }catch{} }
async function fetchRates(){ if(!isAuthenticated.value) return; try{ const r=await axios.get('/api/v1/admin/rates',{headers:authHeaders()}); rates.value=r.data }catch{} }
async function refreshRates(){ if(!isAuthenticated.value) return; try{ await axios.post('/api/v1/admin/rates/refresh',{}, {headers:authHeaders()}); await fetchRates() }catch(e:any){ alert(e?.response?.data?.error||'Failed') } }
async function saveFee(f:any){ try{ await axios.post(`/api/v1/admin/fees/${f.fee_type}`, {fixed_amount: Number(f.fixed), percent: Number(f.percent), is_active: f.active}, {headers:authHeaders()}); await fetchFees() }catch(e:any){ alert(e?.response?.data?.error||'Failed') } }
async function fetchKB() {
  try { const r = await axios.get('/api/v1/admin/kb', { params:{ q: kbSearch.value } }); kbItems.value=r.data } catch {}
}
async function fetchRoles(){
  if(!isAuthenticated.value) return
  try { const r = await axios.get('/api/v1/admin/roles', { headers: authHeaders() }); roles.value=r.data } catch {}
}
async function fetchAll(){
  await Promise.all([fetchDashboard(), fetchBotStats(), fetchBotInteractions(), fetchEmployees(), fetchKB(), fetchRoles(), fetchTransactions(), fetchForeign(), fetchFees(), fetchRates()])
}

onMounted(async()=>{
  if(isAuthenticated.value) await fetchAll()
  else await fetchDashboard()
  pollTimer = window.setInterval(()=>{ fetchDashboard(); fetchBotStats(); fetchTransactions(); }, 5000)
})
onUnmounted(()=>{ if(pollTimer) clearInterval(pollTimer) })

// ===== Actions =====
const resolveTrade = async(trade:any, status:string)=>{
  let reason=''
  if(status==='Rejected'||status==='reject'){ const inp=window.prompt('Rejection reason:','Card has already been redeemed.'); if(inp===null) return; reason=inp }
  const orig=trade.status; trade.status='Processing...'
  try{
    await axios.post(`/api/v1/admin/trades/${trade.db_id}/resolve`, { status, action: status.toLowerCase(), reason }, { headers: authHeaders() })
    trade.status = status.toLowerCase().includes('approve') ? 'Approved' : 'Rejected'
    setTimeout(fetchDashboard, 800)
  }catch(e:any){ alert(e?.response?.data?.error||'Failed'); trade.status=orig }
}
async function createCatalogue(){
  try{ await axios.post('/api/v1/admin/catalogue', { brand:newCat.value.brand, country:newCat.value.country, card_format:newCat.value.card_format, rate_per_dollar: Number(newCat.value.rate_per_dollar), active:newCat.value.active }, { headers: authHeaders() }); showCatCreate.value=false; newCat.value={brand:'',country:'US',card_format:'PHYSICAL',rate_per_dollar:0,active:true}; await fetchDashboard() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
async function deleteCatalogue(id:number){
  if(!confirm('Delete catalogue entry?')) return
  try{ await axios.delete(`/api/v1/admin/catalogue/${id}`, { headers: authHeaders() }); await fetchDashboard() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
async function saveEditCat(){
  if(!editingCat.value) return
  try{ await axios.post(`/api/v1/admin/catalogue/${editingCat.value.id}`, { brand:editingCat.value.brand, country:editingCat.value.country, card_format:editingCat.value.type||editingCat.value.card_format, rate_per_dollar: Number(editingCat.value.ratePerDollar), active: editingCat.value.status==='Active' }, { headers: authHeaders() }); editingCat.value=null; await fetchDashboard() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
async function createEmployee(){
  try{ await axios.post('/api/v1/admin/employees', newEmp.value, { headers: authHeaders() }); showEmpCreate.value=false; newEmp.value={email:'',password:'',full_name:'',role:'SUPPORT_AGENT'}; await fetchEmployees() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
async function deleteEmployee(id:string){
  if(!confirm('Deactivate employee?')) return
  try{ await axios.delete(`/api/v1/admin/employees/${id}`, { headers: authHeaders() }); await fetchEmployees() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
async function resolveInteraction(id:string){
  try{ await axios.post(`/api/v1/admin/bot/interactions/${id}/resolve`, {}, { headers: authHeaders() }); await fetchBotInteractions() }catch(e:any){ alert(e?.response?.data?.error||'Failed') }
}
</script>

<template>
  <!-- Login -->
  <div v-if="!isAuthenticated" class="min-h-screen bg-obsidian flex items-center justify-center p-6 font-sans">
    <div class="w-full max-w-md bg-obsidian-card border border-obsidian-border rounded-2xl p-8 shadow-2xl">
      <div class="text-center mb-8">
        <div class="w-12 h-12 mx-auto rounded-xl bg-gradient-to-br from-navy-accent to-silver-metallic flex items-center justify-center font-bold text-obsidian text-xl">M</div>
        <h1 class="text-xl font-bold text-silver-light mt-3">MiddleMan Admin</h1>
        <p class="text-xs text-silver-muted">10Alytics BuildFest 2026 — Case Study 1: AI Customer Support</p>
        <p class="text-[11px] text-silver-dark mt-2">Demo super_admin: <b>garbaabdullahi344@gmail.com</b> / <b>Babawo_344</b></p>
      </div>
      <div class="space-y-4">
        <input v-model="loginEmail" placeholder="Email" class="w-full px-4 py-3 rounded-lg bg-obsidian-dark border border-obsidian-border text-silver-light text-sm focus:outline-none focus:border-silver-metallic"/>
        <input v-model="loginPassword" type="password" placeholder="Password" @keyup.enter="doLogin" class="w-full px-4 py-3 rounded-lg bg-obsidian-dark border border-obsidian-border text-silver-light text-sm focus:outline-none focus:border-silver-metallic"/>
        <p v-if="loginError" class="text-xs text-red-400">{{ loginError }}</p>
        <button @click="doLogin" :disabled="loginLoading" class="w-full py-3 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white font-semibold rounded-lg text-sm">{{ loginLoading ? 'Signing in…' : 'Sign In' }}</button>
        <p class="text-[11px] text-silver-dark text-center">Employees: SUPPORT_AGENT / OPERATIONS_MANAGER / COMPLIANCE — ask super_admin to create account</p>
      </div>
    </div>
  </div>

  <!-- App -->
  <div v-else class="min-h-screen bg-obsidian text-silver-light flex font-sans">
    <!-- Sidebar -->
    <aside class="w-72 bg-obsidian-dark border-r border-obsidian-border flex flex-col justify-between shrink-0">
      <div>
        <div class="p-6 border-b border-obsidian-border bg-navy-dark">
          <div class="flex items-center space-x-3">
            <div class="w-10 h-10 rounded-lg bg-gradient-to-br from-navy-accent to-silver-metallic flex items-center justify-center font-bold text-obsidian text-xl shadow-lg">M</div>
            <div><h1 class="text-lg font-bold tracking-wide">MiddleMan</h1><p class="text-xs text-silver-muted">AI Support Control</p></div>
          </div>
          <div class="mt-3 px-3 py-2 rounded-lg bg-obsidian-card border border-obsidian-border">
            <p class="text-xs text-silver-muted">Signed in as</p><p class="text-sm font-semibold">{{ currentUser?.full_name || currentUser?.email }}</p><p class="text-[11px] px-2 py-0.5 rounded bg-navy text-silver-light inline-block mt-1">{{ currentUser?.role }}</p>
          </div>
        </div>
        <nav class="p-4 space-y-1">
          <button v-for="item in navigation" :key="item.name" @click="activeTab=item.name" :class="['w-full flex items-center space-x-3 px-4 py-3 rounded-lg text-sm font-medium transition-all', activeTab===item.name ? 'bg-navy text-silver-light border-l-4 border-silver-metallic shadow-md' : 'text-silver-muted hover:bg-obsidian-card hover:text-silver-light']"><span>{{ item.icon }}</span><span>{{ item.name }}</span></button>
        </nav>
      </div>
      <div class="p-4 border-t border-obsidian-border"><button @click="logout" class="w-full py-2 rounded-lg bg-red-600/80 hover:bg-red-500 text-white text-xs font-semibold">Logout</button></div>
    </aside>

    <!-- Main -->
    <main class="flex-1 flex flex-col h-screen overflow-hidden bg-obsidian">
      <header class="bg-navy-dark border-b border-obsidian-border px-8 py-4 flex justify-between items-center shrink-0">
        <div class="flex items-center space-x-4"><h2 class="text-xl font-bold">{{ activeTab }}</h2><span :class="['px-3 py-1 rounded-full text-xs border', isLive?'bg-emerald-500/10 text-emerald-400 border-emerald-500/30':'bg-red-500/10 text-red-400 border-red-500/30']">{{ isLive?'● Live':'○ Offline' }}</span></div>
        <button @click="fetchAll()" class="text-xs px-3 py-1 rounded bg-obsidian-card border border-obsidian-border text-silver-muted">↻ Refresh</button>
      </header>

      <div class="flex-1 overflow-auto p-8 space-y-6">
        <!-- Analytics -->
        <div v-if="activeTab==='Analytics & Bot Metrics'" class="space-y-6">
          <div v-if="!botStats" class="text-silver-muted">Loading analytics…</div>
          <template v-else>
            <div class="grid grid-cols-2 lg:grid-cols-4 gap-4">
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Total Interactions</p><p class="text-2xl font-bold mt-1">{{ botStats.total_interactions }}</p><p class="text-[11px] text-silver-dark">{{ botStats.today_interactions }} today</p></div>
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Escalation Rate</p><p class="text-2xl font-bold mt-1 text-amber-400">{{ botStats.escalation_rate.toFixed(1) }}%</p><p class="text-[11px] text-silver-dark">{{ botStats.escalated_count }} escalated</p></div>
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Auto-Resolved</p><p class="text-2xl font-bold mt-1 text-emerald-400">{{ botStats.auto_resolved }}</p><p class="text-[11px] text-silver-dark">via AI + KB</p></div>
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Avg Handling</p><p class="text-2xl font-bold mt-1">{{ (botStats.avg_handling_ms/1000).toFixed(1) }}s</p><p class="text-[11px] text-silver-dark">bot response time</p></div>
            </div>
            <!-- 14-day trend -->
            <div class="bg-obsidian-card p-6 rounded-xl border border-obsidian-border">
              <h3 class="text-sm font-semibold mb-3">Messages — Last 14 Days (bot_analytics)</h3>
              <div class="flex items-end gap-1 h-24">
                <div v-for="d in botStats.last_14_days" :key="d.date" class="flex-1 flex flex-col items-center gap-1">
                  <div :style="{ height: (d.value/320*80)+'px' }" class="w-full bg-gradient-to-t from-navy-accent to-emerald-500/70 rounded-t min-h-[4px]"></div>
                  <span class="text-[9px] text-silver-dark">{{ d.date.slice(5) }}</span>
                </div>
              </div>
            </div>
            <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border">
                <h4 class="text-xs font-semibold text-silver-muted mb-3">By Category (classification)</h4>
                <div v-for="c in botStats.by_category" :key="c.name" class="flex items-center gap-2 mb-2"><span class="text-xs w-28 truncate">{{ c.name }}</span><div class="flex-1 h-2 bg-obsidian-dark rounded overflow-hidden"><div :style="{width:(c.value/Math.max(...botStats.by_category.map((x:any)=>x.value))*100)+'%'}" class="h-full bg-silver-metallic"></div></div><span class="text-xs w-6">{{ c.value }}</span></div>
              </div>
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border">
                <h4 class="text-xs font-semibold text-silver-muted mb-3">By Sentiment</h4>
                <div v-for="s in botStats.by_sentiment" :key="s.name" class="flex items-center gap-2 mb-2"><span :class="['text-xs w-20 px-2 py-1 rounded', s.name==='negative'?'bg-red-500/20 text-red-400': s.name==='positive'?'bg-emerald-500/20 text-emerald-400':'bg-zinc-500/20']">{{ s.name }}</span><div class="flex-1 h-2 bg-obsidian-dark rounded overflow-hidden"><div :style="{width:(s.value/botStats.total_interactions*100)+'%'}" :class="['h-full', s.name==='negative'?'bg-red-500': s.name==='positive'?'bg-emerald-500':'bg-zinc-500']"></div></div><span class="text-xs">{{ s.value }}</span></div>
              </div>
              <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border">
                <h4 class="text-xs font-semibold text-silver-muted mb-3">By Urgency</h4>
                <div v-for="u in botStats.by_urgency" :key="u.name" class="flex items-center gap-2 mb-2"><span :class="['text-xs w-20 px-2 py-1 rounded', u.name==='critical'?'bg-red-600 text-white': u.name==='high'?'bg-amber-500/20 text-amber-400': u.name==='medium'?'bg-yellow-500/20 text-yellow-400':'bg-zinc-500/20']">{{ u.name }}</span><div class="flex-1 h-2 bg-obsidian-dark rounded"><div :style="{width:(u.value/botStats.total_interactions*100)+'%'}" class="h-full bg-amber-500"></div></div><span class="text-xs">{{ u.value }}</span></div>
              </div>
            </div>
            <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border">
              <h4 class="text-xs font-semibold text-silver-muted mb-2">Top Intents (AI NLU)</h4>
              <div class="flex flex-wrap gap-2"><span v-for="it in botStats.by_intent" :key="it.name" class="px-3 py-1 rounded-full bg-navy border border-obsidian-border text-xs">{{ it.name }} <b class="text-emerald-400">{{ it.value }}</b></span></div>
            </div>
            <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border">
              <p class="text-xs text-silver-muted">DB tables powering this view: <code class="bg-obsidian-dark px-1 rounded">bot_interactions</code> (120 rows), <code class="bg-obsidian-dark px-1">bot_analytics</code> (14 days), <code class="bg-obsidian-dark px-1">knowledge_base</code> (10 articles), <code class="bg-obsidian-dark px-1">gift_card_trades</code>, <code class="bg-obsidian-dark px-1">users</code>. Case Study 1 flow: intake → classify → sentiment/urgency → KB retrieve → escalate → capture.</p>
            </div>
          </template>
        </div>

        <!-- Bot Inbox -->
        <div v-if="activeTab==='Bot Inbox'" class="space-y-4">
          <div class="flex gap-2"><button @click="fetchBotInteractions(false)" class="px-3 py-1 rounded bg-navy text-xs border border-obsidian-border">All</button><button @click="fetchBotInteractions(true)" class="px-3 py-1 rounded bg-amber-600 text-xs text-white">Escalated only ({{ botStats?.escalated_count || 0 }})</button></div>
          <div v-for="m in botInteractions" :key="m.id" class="bg-obsidian-card border border-obsidian-border rounded-xl p-4">
            <div class="flex justify-between"><span class="text-[11px] font-mono text-silver-dark">{{ m.whatsapp_number }} • {{ new Date(m.created_at).toLocaleString() }}</span><span :class="['text-[11px] px-2 py-1 rounded', m.escalated?'bg-red-600 text-white':'bg-emerald-600/20 text-emerald-400']">{{ m.escalated ? 'ESCALATED: '+m.escalation_reason : 'auto-resolved' }}</span></div>
            <p class="text-sm mt-2">"{{ m.inbound_text }}"</p>
            <div class="flex flex-wrap gap-2 mt-2 text-[11px]"><span class="px-2 py-1 rounded bg-obsidian-dark border">intent: {{ m.intent }}</span><span class="px-2 py-1 rounded bg-obsidian-dark border">cat: {{ m.category }}</span><span :class="['px-2 py-1 rounded', m.sentiment==='negative'?'bg-red-500/20 text-red-400':'bg-zinc-500/20']">{{ m.sentiment }}</span><span class="px-2 py-1 rounded bg-amber-500/20">{{ m.urgency }} ({{ m.urgency_score }})</span><span class="px-2 py-1 rounded">conf {{ Number(m.confidence).toFixed(2) }}</span><span class="px-2 py-1 rounded">{{ m.handling_ms }}ms</span></div>
            <p class="text-xs text-silver-muted mt-2">→ {{ m.response_text }}</p>
            <button v-if="m.escalated && !m.resolved" @click="resolveInteraction(m.id)" class="mt-3 px-3 py-1 bg-emerald-600 text-white rounded text-xs">Mark Resolved</button><span v-if="m.resolved" class="mt-3 inline-block px-3 py-1 bg-zinc-600 text-white rounded text-xs">Resolved</span>
          </div>
        </div>

        <!-- Trades -->
        <div v-if="activeTab==='Gift Card Trades'" class="space-y-6">
          <div class="grid grid-cols-3 gap-4">
            <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Pending</p><p class="text-2xl font-bold">{{ dashboardStats.pendingCards }}</p></div>
            <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Today Volume</p><p class="text-2xl font-bold">{{ dashboardStats.todayVolume }}</p></div>
            <div class="bg-obsidian-card p-5 rounded-xl border border-obsidian-border"><p class="text-xs text-silver-muted">Active Users</p><p class="text-2xl font-bold text-emerald-400">{{ dashboardStats.activeUsers }}</p></div>
          </div>
          <div v-if="giftcardTrades.length===0" class="text-center py-12 border border-dashed rounded-xl bg-obsidian-card/50 text-silver-muted text-sm">No trades yet</div>
          <div v-else class="grid grid-cols-1 lg:grid-cols-2 gap-4">
            <div v-for="t in giftcardTrades" :key="t.id" class="bg-obsidian-card border border-obsidian-border rounded-xl p-5 space-y-3">
              <div class="flex justify-between"><div><span class="text-xs font-mono text-silver-dark">{{ t.id }}</span><h4 class="font-bold">{{ t.card }} ({{ t.amount }})</h4><p class="text-xs text-silver-muted">User: {{ t.user }}</p></div><span :class="['px-3 py-1 rounded-full text-xs', t.status==='Pending Review'?'bg-amber-500/10 text-amber-400 border': t.status==='Approved'?'bg-emerald-500/10 text-emerald-400 border':'bg-red-500/10 text-red-400 border']">{{ t.status }}</span></div>
              <div class="p-3 bg-obsidian-dark rounded border flex justify-between items-center"><div><p class="text-xs text-silver-muted">Payout</p><p class="font-bold text-emerald-400">{{ t.calculatedNaira }}</p></div><button @click="t.image_url?selectedImage=t.image_url:null" :disabled="!t.image_url" class="px-3 py-1 text-xs rounded border" :class="t.image_url?'bg-navy text-silver-light':'opacity-30'">Inspect</button></div>
              <div class="flex gap-2"><button @click="resolveTrade(t,'Approved')" :disabled="t.status==='Approved'||t.status==='Rejected'" class="flex-1 py-2 bg-emerald-600 text-white rounded text-xs disabled:opacity-30">Approve & Pay</button><button @click="resolveTrade(t,'Rejected')" :disabled="t.status==='Approved'||t.status==='Rejected'" class="flex-1 py-2 bg-red-600/80 text-white rounded text-xs disabled:opacity-30">Reject</button></div>
            </div>
          </div>
        </div>

        <!-- Catalogue -->
        <div v-if="activeTab==='Price Catalogue'" class="space-y-4">
          <button @click="showCatCreate=!showCatCreate" class="px-4 py-2 bg-emerald-600 text-white rounded text-xs">+ New Rate</button>
          <div v-if="showCatCreate" class="bg-obsidian-card p-4 rounded-xl border border-obsidian-border flex flex-wrap gap-2">
            <input v-model="newCat.brand" placeholder="Brand (e.g. STEAM)" class="px-3 py-2 rounded bg-obsidian-dark border border-obsidian-border text-sm"/><input v-model="newCat.country" placeholder="Country" class="px-3 py-2 rounded bg-obsidian-dark border text-sm w-20"/><input v-model="newCat.card_format" placeholder="PHYSICAL/ECODE" class="px-3 py-2 rounded bg-obsidian-dark border text-sm w-32"/><input v-model.number="newCat.rate_per_dollar" type="number" placeholder="Rate" class="px-3 py-2 rounded bg-obsidian-dark border text-sm w-24"/><button @click="createCatalogue" class="px-4 py-2 bg-navy text-white rounded text-xs">Save</button>
          </div>
          <div class="bg-obsidian-card border rounded-xl overflow-hidden">
            <table class="w-full text-left text-sm">
              <thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">Brand</th><th class="p-3">Country</th><th class="p-3">Type</th><th class="p-3">Rate ₦/$</th><th class="p-3">Status</th><th class="p-3"></th></tr></thead>
              <tbody class="divide-y divide-obsidian-border">
                <tr v-for="it in priceCatalogue" :key="it.id" class="hover:bg-obsidian-dark/50">
                  <template v-if="editingCat?.id===it.id">
                    <td class="p-2"><input v-model="editingCat.brand" class="px-2 py-1 rounded bg-obsidian-dark border text-sm w-24"/></td>
                    <td class="p-2"><input v-model="editingCat.country" class="px-2 py-1 rounded bg-obsidian-dark border text-sm w-16"/></td>
                    <td class="p-2"><input v-model="editingCat.type" class="px-2 py-1 rounded bg-obsidian-dark border text-sm w-20"/></td>
                    <td class="p-2"><input v-model.number="editingCat.ratePerDollar" type="number" class="px-2 py-1 rounded bg-obsidian-dark border text-sm w-20"/></td>
                    <td class="p-2"><select v-model="editingCat.status" class="px-2 py-1 rounded bg-obsidian-dark border text-sm"><option>Active</option><option>Inactive</option></select></td>
                    <td class="p-2 flex gap-1"><button @click="saveEditCat" class="px-2 py-1 bg-emerald-600 text-white rounded text-xs">Save</button><button @click="editingCat=null" class="px-2 py-1 bg-zinc-600 text-white rounded text-xs">Cancel</button></td>
                  </template>
                  <template v-else>
                    <td class="p-3 font-semibold">{{ it.brand }}</td><td class="p-3">{{ it.country }}</td><td class="p-3">{{ it.type }}</td><td class="p-3 font-mono text-emerald-400">₦{{ it.ratePerDollar }}</td><td class="p-3"><span :class="it.status==='Active'?'text-emerald-400':'text-red-400'">● {{ it.status }}</span></td><td class="p-3 flex gap-1"><button @click="editingCat={...it}" class="px-2 py-1 bg-navy text-white rounded text-xs">Edit</button><button @click="deleteCatalogue(it.id)" class="px-2 py-1 bg-red-600 text-white rounded text-xs">Del</button></td>
                  </template>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <!-- Knowledge Base -->
        <div v-if="activeTab==='Knowledge Base'" class="space-y-4">
          <input v-model="kbSearch" @input="fetchKB" placeholder="Search knowledge base…" class="w-full px-4 py-3 rounded-lg bg-obsidian-card border border-obsidian-border text-sm"/>
          <div v-for="k in kbItems" :key="k.id" class="bg-obsidian-card border border-obsidian-border rounded-xl p-4"><p class="text-xs px-2 py-1 rounded bg-navy inline-block">{{ k.category }}</p><h4 class="font-semibold mt-2">{{ k.question }}</h4><p class="text-sm text-silver-muted mt-1">{{ k.answer }}</p></div>
        </div>

        <!-- Transactions -->
        <div v-if="activeTab==='Transactions'" class="space-y-4">
          <div class="bg-obsidian-card border rounded-xl overflow-hidden">
            <table class="w-full text-left text-sm"><thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">Type</th><th class="p-3">Amount</th><th class="p-3">Currency</th><th class="p-3">Status</th><th class="p-3">Time</th></tr></thead>
              <tbody class="divide-y"><tr v-for="t in transactions" :key="t.id"><td class="p-3">{{ t.tx_type }}</td><td class="p-3 font-mono">{{ t.amount }}</td><td class="p-3">{{ t.currency }}</td><td class="p-3">{{ t.status }}</td><td class="p-3 text-xs">{{ new Date(t.created_at).toLocaleString() }}</td></tr></tbody>
            </table>
          </div>
          <p class="text-xs text-silver-muted">Shows all: P2P, FIAT_PAYOUT (external bank), AIRTIME, CRYPTO_TRANSFER, OFFRAMP, GIFT_CARD_PAYOUT. Filter via DB <code>transactions</code>.</p>
        </div>

        <!-- Fees -->
        <div v-if="activeTab==='Fees & Charges'" class="space-y-4">
          <div class="bg-obsidian-card p-4 rounded-xl border">
            <h3 class="text-sm font-semibold">Platform fees — editable (applies next transaction). Giftcard uses manual price_catalogue, not this table.</h3>
            <p class="text-xs text-silver-muted mt-1">Fee = fixed + percent*amount. Crypto onchain adds gas (~0.0005 SOL) on top. Spot 0.8% | Futures 1% | Degen 1.5% | Offramp 1.2% | Fiat payout 50+0.5% .</p>
          </div>
          <div class="bg-obsidian-card border rounded-xl overflow-hidden">
            <table class="w-full text-left text-sm"><thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">Fee Type</th><th class="p-3">Fixed</th><th class="p-3">Percent %</th><th class="p-3">Currency</th><th class="p-3">Active</th><th class="p-3"></th></tr></thead>
              <tbody class="divide-y"><tr v-for="f in fees" :key="f.fee_type">
                <td class="p-3 font-mono text-xs">{{ f.fee_type }}</td>
                <td class="p-3"><input v-model="f.fixed" type="number" step="0.01" class="w-20 px-2 py-1 rounded bg-obsidian-dark border text-sm"/></td>
                <td class="p-3"><input v-model="f.percent" type="number" step="0.01" class="w-16 px-2 py-1 rounded bg-obsidian-dark border text-sm"/></td>
                <td class="p-3 text-xs">{{ f.currency }}</td>
                <td class="p-3"><input type="checkbox" v-model="f.active"/></td>
                <td class="p-3"><button @click="saveFee(f)" class="px-3 py-1 bg-emerald-600 text-white rounded text-xs">Save</button></td>
              </tr></tbody>
            </table>
          </div>
        </div>

        <!-- Rates -->
        <div v-if="activeTab==='Rates (Auto)'" class="space-y-4">
          <div class="bg-obsidian-card p-4 rounded-xl border flex justify-between items-center">
            <div><h3 class="text-sm font-semibold">Auto rates — non-giftcard</h3><p class="text-xs text-silver-muted">Giftcard rates stay manual via Price Catalogue. This table auto-updates: crypto 300s (coingecko), fiat 3600s (exchangerate-api/frankfurter), fallback ±2% jitter if offline. Last 8 pairs.</p></div>
            <button @click="refreshRates" class="px-4 py-2 bg-navy border text-white rounded text-xs">↻ Refresh now</button>
          </div>
          <div class="bg-obsidian-card border rounded-xl overflow-hidden">
            <table class="w-full text-left text-sm"><thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">Pair</th><th class="p-3">Source</th><th class="p-3">Mid Rate</th><th class="p-3">Last Fetched</th><th class="p-3">Auto</th><th class="p-3">Status</th></tr></thead>
              <tbody class="divide-y"><tr v-for="r in rates" :key="r.pair">
                <td class="p-3 font-mono">{{ r.pair }}</td><td class="p-3 text-xs">{{ r.source }}</td><td class="p-3 font-mono text-emerald-400">{{ r.current_mid ?? r.last_rate ?? '—' }}</td><td class="p-3 text-xs">{{ r.current_fetched ?? r.last_fetched ?? '—' }}</td><td class="p-3">{{ r.auto ? 'ON' : 'OFF' }}</td><td class="p-3 text-xs" :class="r.last_error ? 'text-red-400' : 'text-emerald-400'">{{ r.last_error || 'OK' }}</td>
              </tr></tbody>
            </table>
          </div>
          <p class="text-xs text-silver-muted">Dashboard polls every 5s; rates tick every 60s in backend <code>rates::spawn</code>. Manual giftcard KYC: see Price Catalogue.</p>
        </div>

        <!-- Foreign Accounts -->
        <div v-if="activeTab==='Foreign Accounts'" class="space-y-4">
          <div class="bg-obsidian-card p-4 rounded-xl border">
            <p class="text-sm font-semibold">For freelancers & remote workers — mock USD/GBP/EUR virtual accounts (future: Wise/Stripe). Create via WhatsApp: <code>create USD account</code></p>
            <button @click="fetchForeign" class="mt-2 px-3 py-1 rounded bg-navy text-xs border">Refresh</button>
          </div>
          <div class="bg-obsidian-card border rounded-xl overflow-hidden">
            <table class="w-full text-left text-sm"><thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">User</th><th class="p-3">Currency</th><th class="p-3">Account</th><th class="p-3">Provider</th><th class="p-3">Status</th></tr></thead>
              <tbody class="divide-y"><tr v-for="f in foreignAccts" :key="f.id"><td class="p-3">{{ f.user }}</td><td class="p-3">{{ f.currency }}</td><td class="p-3 font-mono">{{ f.account }}</td><td class="p-3">{{ f.provider }}</td><td class="p-3">{{ f.status }}</td></tr></tbody>
            </table>
          </div>
          <div class="bg-obsidian-card p-4 rounded-xl border text-xs text-silver-muted">Security: these wallets are <code>wallets(currency=USD/GBP/EUR)</code> + <code>foreign_accounts</code>, isolated per user, provider mock. Real provider adds KYC gate <code>users.kyc_status</code>.<br/>WhatsApp security: PIN 4-6 digits (Argon2id), cached 15min, step-up OTP for &gt;₦100k, velocity 5/h &amp; ₦500k/day, lockout 2min/15min, Redis <code>pin_ok:</code>/<code>otp:</code>, all keys AES-256-GCM.</div>
        </div>

        <!-- Employees -->
        <div v-if="activeTab==='Employee Management'" class="space-y-4">
          <div v-if="currentUser?.role!=='SUPER_ADMIN'" class="p-6 bg-red-500/10 border border-red-500/30 rounded-xl text-sm">Only SUPER_ADMIN can manage employees. Your role: {{ currentUser?.role }}</div>
          <template v-else>
            <button @click="showEmpCreate=!showEmpCreate" class="px-4 py-2 bg-emerald-600 text-white rounded text-xs">+ New Employee</button>
            <div v-if="showEmpCreate" class="bg-obsidian-card p-4 rounded-xl border flex flex-wrap gap-2">
              <input v-model="newEmp.email" placeholder="Email" class="px-3 py-2 rounded bg-obsidian-dark border text-sm"/><input v-model="newEmp.password" type="password" placeholder="Password" class="px-3 py-2 rounded bg-obsidian-dark border text-sm"/><input v-model="newEmp.full_name" placeholder="Full name" class="px-3 py-2 rounded bg-obsidian-dark border text-sm"/><select v-model="newEmp.role" class="px-3 py-2 rounded bg-obsidian-dark border text-sm"><option>SUPPORT_AGENT</option><option>OPERATIONS_MANAGER</option><option>COMPLIANCE</option><option>AGENT</option></select><button @click="createEmployee" class="px-4 py-2 bg-navy text-white rounded text-xs">Create</button>
            </div>
            <div class="bg-obsidian-card border rounded-xl overflow-hidden">
              <table class="w-full text-left text-sm"><thead class="bg-navy-dark text-silver-muted text-xs uppercase"><tr><th class="p-3">Email</th><th class="p-3">Name</th><th class="p-3">Role</th><th class="p-3">Active</th><th class="p-3"></th></tr></thead>
                <tbody class="divide-y"><tr v-for="e in employees" :key="e.id"><td class="p-3">{{ e.email }}</td><td class="p-3">{{ e.full_name }}</td><td class="p-3"><span class="px-2 py-1 rounded bg-navy text-xs">{{ e.role }}</span></td><td class="p-3">{{ e.is_active?'Yes':'No' }}</td><td class="p-3"><button @click="deleteEmployee(e.id)" class="px-2 py-1 bg-red-600 text-white rounded text-xs">Deactivate</button></td></tr></tbody>
              </table>
            </div>
            <div class="bg-obsidian-card p-4 rounded-xl border">
              <h4 class="text-xs font-semibold mb-2">Role Permissions</h4>
              <div v-for="r in roles" :key="r.role" class="mb-3"><p class="text-xs font-bold">{{ r.role }}</p><div class="flex flex-wrap gap-1 mt-1"><span v-for="p in r.permissions" :key="p.permission" class="text-[10px] px-2 py-1 rounded bg-obsidian-dark border">{{ p.permission }}</span></div></div>
            </div>
          </template>
        </div>

        <!-- Audit Logs placeholder -->
        <div v-if="activeTab==='Audit Logs'" class="bg-obsidian-card p-6 rounded-xl border border-obsidian-border text-sm text-silver-muted">Audit logs: tracked in <code>admin_audit_logs</code> and <code>price_catalogue_audit</code>. Query via DB. Escalated bot interactions auto-log to audit trail.</div>
      </div>
    </main>

    <div v-if="selectedImage" class="absolute inset-0 z-50 flex items-center justify-center bg-obsidian-dark/80 backdrop-blur-sm p-6" @click="selectedImage=null"><img :src="selectedImage!" class="max-w-2xl max-h-[80vh] rounded-lg border shadow-2xl"/></div>
  </div>
</template>
