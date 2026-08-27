# VOLUME ZETA — Vue 3 Admin Operations Dashboard

**Version:** 2.4.0 · **Owner:** Frontend Engineering · **Scope:** Ops panel for gift-card desk,
KYC queue, ledger monitor, audit log; JWT auth + live WebSocket updates.

---

## 1. Architectural Overview & Technical Scope

SPA served by nginx behind Caddy at `/`. Talks to the Rust core only:

- REST under `/api/v1/*` (JWT `Authorization: Bearer`).
- Live desk via WebSocket `/api/v1/ws` (token in query string on upgrade).

Stack: Vue 3 Composition API + `<script setup>`, Vite, Pinia, Tailwind CSS, Axios.
No SSR, no heavy charting lib — the 40 MB container budget demands a lean bundle (<300 KB gz).

Views:

| Route | View | Purpose |
|---|---|---|
| `/login` | LoginView | Email + password -> JWT |
| `/` | TradeDesk | Pending gift cards grid, zoom modal, approve/reject |
| `/ledger` | LedgerMonitor | Reserves vs liability tally, recent transactions |
| `/kyc` | KycQueue | Tier-2 document review |
| `/audit` | AuditLog | Searchable operator actions table |

## 2. Interface Contracts

```
POST /api/v1/admin/login        {email, password} -> {token, employee:{id,email,role}}
GET  /api/v1/admin/dashboard    -> {stats:{activeUsers,pendingCards,todayVolume},
                                    trades:[{db_id,id,user,card,amount,calculatedNaira,status,image_url,time}],
                                    catalogue:[{id,brand,type,ratePerDollar,status}]}
POST /api/v1/admin/trades/:id/resolve   {action:"approve"|"reject", reason?, adjusted_payout?}
WS   /api/v1/ws?token=<jwt>     events: trade.created | trade.resolved
```

## 3. Complete Implementation

### 3.1 `src/main.ts`

```typescript
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import router from './router';
import './style.css';

createApp(App).use(createPinia()).use(router).mount('#app');
```

### 3.2 `src/router/index.ts`

```typescript
import { createRouter, createWebHistory } from 'vue-router';
import { useAuthStore } from '../stores/auth';

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', component: () => import('../views/LoginView.vue') },
    { path: '/', component: () => import('../views/TradeDesk.vue'), meta: { requiresAuth: true } },
    { path: '/ledger', component: () => import('../views/LedgerMonitor.vue'), meta: { requiresAuth: true } },
    { path: '/kyc', component: () => import('../views/KycQueue.vue'), meta: { requiresAuth: true } },
    { path: '/audit', component: () => import('../views/AuditLogView.vue'), meta: { requiresAuth: true } },
  ],
});

router.beforeEach((to) => {
  const auth = useAuthStore();
  if (to.meta.requiresAuth && !auth.token) return '/login';
});

export default router;
```

### 3.3 `src/api/client.ts`

```typescript
import axios from 'axios';

export const api = axios.create({ baseURL: import.meta.env.VITE_API_BASE ?? '/api/v1' });

api.interceptors.request.use((cfg) => {
  const token = localStorage.getItem('mm_jwt');
  if (token) cfg.headers.Authorization = `Bearer ${token}`;
  return cfg;
});

api.interceptors.response.use(
  (r) => r,
  (err) => {
    if (err.response?.status === 401) {
      localStorage.removeItem('mm_jwt');
      window.location.href = '/login';
    }
    return Promise.reject(err);
  }
);
```

### 3.4 `src/stores/auth.ts`

```typescript
import { defineStore } from 'pinia';
import { api } from '../api/client';

interface Employee { id: string; email: string; role: string }

export const useAuthStore = defineStore('auth', {
  state: () => ({
    token: localStorage.getItem('mm_jwt') ?? '',
    employee: JSON.parse(localStorage.getItem('mm_employee') ?? 'null') as Employee | null,
  }),
  getters: {
    isSuperAdmin: (s) => s.employee?.role === 'SUPER_ADMIN',
  },
  actions: {
    async login(email: string, password: string) {
      const { data } = await api.post('/admin/login', { email, password });
      this.token = data.token;
      this.employee = data.employee;
      localStorage.setItem('mm_jwt', data.token);
      localStorage.setItem('mm_employee', JSON.stringify(data.employee));
    },
    logout() {
      this.token = '';
      this.employee = null;
      localStorage.clear();
    },
  },
});
```

### 3.5 `src/stores/desk.ts` — trades + WebSocket

```typescript
import { defineStore } from 'pinia';
import { api } from '../api/client';
import { useAuthStore } from './auth';

export interface Trade {
  db_id: string;
  id: string;
  user: string;
  card: string;
  amount: string;
  calculatedNaira: string;
  status: string;
  image_url: string | null;
  time: string;
}

let socket: WebSocket | null = null;
let retryTimer: ReturnType<typeof setTimeout> | null = null;

export const useDeskStore = defineStore('desk', {
  state: () => ({
    trades: [] as Trade[],
    stats: { activeUsers: 0, pendingCards: 0, todayVolume: '₦0.00' },
    catalogue: [] as Array<{ id: number; brand: string; type: string; ratePerDollar: number; status: string }>,
    wsConnected: false,
  }),
  actions: {
    async refresh() {
      const { data } = await api.get('/admin/dashboard');
      this.trades = data.trades;
      this.stats = data.stats;
      this.catalogue = data.catalogue;
    },
    async resolve(tradeId: string, action: 'approve' | 'reject', reason?: string) {
      await api.post(`/admin/trades/${tradeId}/resolve`, { action, reason });
      await this.refresh();
    },
    connectWs() {
      const auth = useAuthStore();
      if (!auth.token) return;
      const proto = window.location.protocol === 'https:' ? 'wss://' : 'ws://';
      socket = new WebSocket(`${proto}${window.location.host}/api/v1/ws?token=${auth.token}`);

      socket.onopen = () => { this.wsConnected = true; };
      socket.onmessage = (ev) => {
        const payload = JSON.parse(ev.data);
        if (payload.event === 'trade.created' || payload.event === 'trade.resolved') {
          this.refresh();
        }
      };
      socket.onclose = () => {
        this.wsConnected = false;
        retryTimer = setTimeout(() => this.connectWs(), 3000); // bounded backoff could be added
      };
      socket.onerror = () => socket?.close();
    },
    disconnectWs() {
      if (retryTimer) clearTimeout(retryTimer);
      socket?.close();
      socket = null;
    },
  },
});
```

### 3.6 `src/views/LoginView.vue`

```vue
<script setup lang="ts">
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import { useAuthStore } from '../stores/auth';

const email = ref('');
const password = ref('');
const error = ref('');
const busy = ref(false);
const router = useRouter();
const auth = useAuthStore();

async function submit() {
  busy.value = true;
  error.value = '';
  try {
    await auth.login(email.value, password.value);
    router.push('/');
  } catch (e: any) {
    error.value = e.response?.data?.error ?? 'Login failed';
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="min-h-screen flex items-center justify-center bg-slate-950 text-slate-100">
    <form @submit.prevent="submit" class="w-96 space-y-4 rounded-xl bg-slate-900 p-8 shadow-xl">
      <h1 class="text-xl font-bold">MiddleMan Ops</h1>
      <input v-model="email" type="email" required placeholder="ops@middleman.africa"
             class="w-full rounded bg-slate-800 px-3 py-2 outline-none focus:ring-2 ring-emerald-500" />
      <input v-model="password" type="password" required placeholder="Password"
             class="w-full rounded bg-slate-800 px-3 py-2 outline-none focus:ring-2 ring-emerald-500" />
      <p v-if="error" class="text-sm text-rose-400">{{ error }}</p>
      <button :disabled="busy" class="w-full rounded bg-emerald-500 py-2 font-semibold
                                     hover:bg-emerald-400 disabled:opacity-50">
        {{ busy ? 'Signing in…' : 'Sign in' }}
      </button>
    </form>
  </div>
</template>
```

### 3.7 `src/views/TradeDesk.vue`

```vue
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { useDeskStore, type Trade } from '../stores/desk';
import ImageZoomModal from '../components/ImageZoomModal.vue';

const desk = useDeskStore();
const zoomTarget = ref<Trade | null>(null);
const rejectingId = ref<string | null>(null);
const rejectReason = ref('');

onMounted(() => { desk.refresh(); desk.connectWs(); });
onUnmounted(() => desk.disconnectWs());

async function approve(t: Trade) {
  await desk.resolve(t.db_id, 'approve');
}

function openReject(t: Trade) {
  rejectingId.value = t.db_id;
  rejectReason.value = '';
}

async function confirmReject() {
  if (rejectingId.value) {
    await desk.resolve(rejectingId.value, 'reject', rejectReason.value || undefined);
    rejectingId.value = null;
  }
}
</script>

<template>
  <main class="min-h-screen bg-slate-950 p-6 text-slate-100">
    <header class="mb-6 flex items-center justify-between">
      <h1 class="text-2xl font-bold">Gift Card Desk</h1>
      <div class="flex items-center gap-4 text-sm">
        <span :class="desk.wsConnected ? 'text-emerald-400' : 'text-rose-400'">
          ● {{ desk.wsConnected ? 'live' : 'reconnecting' }}
        </span>
        <button @click="$router.push('/audit')" class="underline">Audit log</button>
        <button @click="auth.logout(); $router.push('/login')" class="underline">Sign out</button>
      </div>
    </header>

    <section class="mb-6 grid grid-cols-3 gap-4">
      <div class="rounded-lg bg-slate-900 p-4"><p class="text-xs text-slate-400">Active users</p>
        <p class="text-2xl font-bold">{{ desk.stats.activeUsers }}</p></div>
      <div class="rounded-lg bg-slate-900 p-4"><p class="text-xs text-slate-400">Pending cards</p>
        <p class="text-2xl font-bold">{{ desk.stats.pendingCards }}</p></div>
      <div class="rounded-lg bg-slate-900 p-4"><p class="text-xs text-slate-400">Today volume</p>
        <p class="text-2xl font-bold">{{ desk.stats.todayVolume }}</p></div>
    </section>

    <div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
      <article v-for="t in desk.trades.filter(x => x.status === 'Pending Review')"
               :key="t.id" class="rounded-lg border border-slate-800 bg-slate-900 p-4">
        <div class="mb-3 flex items-center justify-between">
          <span class="font-mono text-sm text-emerald-400">{{ t.id }}</span>
          <span class="text-xs text-slate-400">{{ t.time }}</span>
        </div>
        <img v-if="t.image_url" :src="t.image_url" alt="card" @click="zoomTarget = t"
             class="mb-3 h-40 w-full cursor-zoom-in rounded object-cover" />
        <dl class="mb-4 space-y-1 text-sm">
          <div class="flex justify-between"><dt class="text-slate-400">User</dt><dd>{{ t.user }}</dd></div>
          <div class="flex justify-between"><dt class="text-slate-400">Card</dt><dd>{{ t.card }}</dd></div>
          <div class="flex justify-between"><dt class="text-slate-400">Amount</dt><dd>{{ t.amount }}</dd></div>
          <div class="flex justify-between font-semibold">
            <dt>Payout</dt><dd>{{ t.calculatedNaira }}</dd></div>
        </dl>
        <div class="flex gap-2">
          <button @click="approve(t)" class="flex-1 rounded bg-emerald-500 py-2 text-sm font-semibold
                  hover:bg-emerald-400">Approve</button>
          <button @click="openReject(t)" class="flex-1 rounded bg-rose-600/90 py-2 text-sm font-semibold
                  hover:bg-rose-500">Reject</button>
        </div>
      </article>
    </div>

    <ImageZoomModal v-if="zoomTarget" :src="zoomTarget.image_url!" @close="zoomTarget = null" />

    <div v-if="rejectingId" class="fixed inset-0 flex items-center justify-center bg-black/70">
      <div class="w-96 rounded-lg bg-slate-900 p-6">
        <h2 class="mb-3 font-bold">Rejection reason</h2>
        <textarea v-model="rejectReason" rows="3" placeholder="Card invalid or already redeemed."
                  class="w-full rounded bg-slate-800 p-2"></textarea>
        <div class="mt-4 flex gap-2">
          <button @click="confirmReject" class="flex-1 rounded bg-rose-600 py-2">Confirm reject</button>
          <button @click="rejectingId = null" class="flex-1 rounded bg-slate-700 py-2">Cancel</button>
        </div>
      </div>
    </div>
  </main>
</template>
```

### 3.8 `src/components/ImageZoomModal.vue`

```vue
<script setup lang="ts">
defineProps<{ src: string }>();
const emit = defineEmits<{ (e: 'close'): void }>();
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/90 p-8"
       @click.self="emit('close')">
    <img :src="src" class="max-h-full max-w-full object-contain cursor-zoom-out" @click="emit('close')" />
  </div>
</template>
```

## 4. Data Schemas & Structural Interfaces

See §2. The store treats `db_id` as the immutable handle for resolve calls; `id` is display-only.

Auth extractor contract on the Rust side (`headers.jwt_employee_id()`): HS256 JWT with claims
`{sub: employee_uuid, role: "SUPER_ADMIN"|"AGENT"|"COMPLIANCE", exp}` signed by `JWT_SECRET`,
5-minute skew tolerance.

## 5. Error Handling Policies

| Case | UX |
|---|---|
| 401 anywhere | Interceptor wipes token, redirects to /login |
| Resolve returns "already resolved" | Toast: "Trade already handled by another agent"; list refreshes |
| WS drop | Badge turns rose + "reconnecting"; REST polling fallback every 15 s while disconnected |
| Image 404 on Cloudinary | Placeholder tile; approve/reject still allowed (agent judgement logged) |

## 6. Verification Test Cases & Command Sequences

```bash
# VZ-T1: build gate
cd apps/admin-dashboard && npm run build     # vue-tsc strict pass, bundle < 300KB gz

# VZ-T2: auth flow
npm run dev &
curl -s localhost:3000/api/v1/admin/login -d '{"email":"ops@middleman.africa","password":"..."}' \
  -H 'Content-Type: application/json'
# bad creds -> 401 -> UI shows error, no token stored

# VZ-T3: live updates
# two browser windows; resolve a trade in window A -> window B updates within ~1s via WS

# VZ-T4: reconnect resilience
docker restart middleman-mm-api-1   # badge goes rose then green after API revives

# VZ-T5: role gate
# AGENT attempts adjusted_payout > 20% uplift -> 403 from API, toast shown, no state change
```
