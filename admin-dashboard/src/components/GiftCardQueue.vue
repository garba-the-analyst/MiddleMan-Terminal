<template>
  <div class="mt-6">
    <div v-if="isLoading" class="text-center py-12 text-mmSilver tracking-widest uppercase text-sm animate-pulse">
      Decrypting pending trades...
    </div>

    <div v-else-if="trades.length === 0" class="text-center py-16 bg-card-gradient rounded-xl border border-white/10 shadow-2xl">
      <span class="text-4xl mb-4 block opacity-50">🛡️</span>
      <h3 class="text-xl font-medium text-white tracking-wide">ZERO PENDING TRADES</h3>
      <p class="text-mmSilverDark mt-2">The queue is currently empty.</p>
    </div>

    <div v-else class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8">
      <div v-for="trade in trades" :key="trade.id" class="bg-card-gradient rounded-xl border border-white/20 overflow-hidden flex flex-col transition-all duration-300 hover:border-white shadow-[0_4px_20px_rgba(0,0,0,0.5)]">
        
        <div class="h-48 bg-[#050505] relative border-b border-white/10">
          <a :href="trade.image_url" target="_blank">
            <img :src="trade.image_url" alt="Gift Card" class="w-full h-full object-cover opacity-80 hover:opacity-100 transition-opacity cursor-pointer grayscale-[20%]" />
          </a>
          <span class="absolute top-3 right-3 bg-white text-black text-xs font-bold px-3 py-1 uppercase tracking-wider rounded-sm">
            Pending
          </span>
        </div>

        <div class="p-6 flex-grow">
          <h2 class="text-2xl font-bold text-white mb-1">${{ trade.amount_in_usd }} {{ trade.card_category }}</h2>
          <p class="text-xs text-mmSilver uppercase tracking-widest mb-5">{{ trade.card_type }} | {{ trade.country }}</p>
          
          <div class="bg-black/40 border border-white/5 p-4 rounded-md mb-5">
            <p class="text-sm text-mmSilverDark mb-1">USER IDENTIFIER:</p>
            <p class="text-base font-medium text-white">{{ trade.full_name }}</p>
            <p class="text-xs text-mmSilver mt-1">{{ trade.whatsapp_number.replace('@c.us', '') }}</p>
          </div>

          <div class="flex justify-between items-end mb-2">
            <span class="text-sm text-mmSilver uppercase tracking-wider">Payout Value:</span>
            <span class="text-xl font-bold text-white">₦{{ parseFloat(trade.offered_naira_value).toLocaleString() }}</span>
          </div>
        </div>

        <div class="p-6 pt-0">
          <button @click="approveTrade(trade.id)" class="w-full bg-transparent border-2 border-white hover:bg-white hover:text-black text-white font-bold py-3 px-4 transition-all duration-300 uppercase tracking-widest text-sm">
            Authorize & Fund
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';

const trades = ref([]);
const isLoading = ref(true);

const API_BASE_URL = 'http://localhost:3000/api/admin';

const fetchPendingTrades = async () => {
  isLoading.value = true;
  try {
    const response = await fetch(`${API_BASE_URL}/giftcards/pending`);
    if (!response.ok) throw new Error('Failed to fetch');
    trades.value = await response.json();
  } catch (error) {
    console.error('Error fetching trades:', error);
    alert('System Offline: Could not connect to the MiddleMan core server.');
  } finally {
    isLoading.value = false;
  }
};

const approveTrade = async (tradeId) => {
  const pin = prompt('SECURITY CLEARANCE:\nEnter Admin PIN to authorize fund transfer:');
  if (pin !== '9999') {
    alert('Access Denied: Invalid PIN.');
    return;
  }

  try {
    const response = await fetch(`${API_BASE_URL}/giftcards/approve`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tradeId: tradeId, adminPin: pin })
    });

    const data = await response.json();

    if (response.ok) {
      alert('✅ AUTHORIZED: Funds transferred and user notified.');
      fetchPendingTrades(); 
    } else {
      alert('❌ SYSTEM ERROR: ' + data.error);
    }
  } catch (error) {
    console.error('Approval error:', error);
    alert('Critical system error during authorization.');
  }
};

defineExpose({ fetchPendingTrades });

onMounted(() => {
  fetchPendingTrades();
});
</script>