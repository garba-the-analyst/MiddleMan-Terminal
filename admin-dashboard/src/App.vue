<template>
  <div class="min-h-screen p-8 font-sans">
    <header class="mb-10 flex justify-between items-center max-w-7xl mx-auto border-b border-mmSilver/20 pb-6">
      <div>
        <h1 class="text-3xl font-bold text-white tracking-tight">MIDDLE<span class="text-mmSilver">MAN</span></h1>
        <p class="text-mmSilver text-sm uppercase tracking-widest mt-1">Command Center</p>
      </div>
      
      <button @click="$refs.queue.fetchPendingTrades()" class="bg-transparent border border-white hover:bg-white hover:text-black text-white px-5 py-2 rounded-md font-medium transition-all duration-300 shadow-[0_0_10px_rgba(255,255,255,0.1)]">
        RELOAD DATABANKS
      </button>
    </header>

    <main class="max-w-7xl mx-auto">
      <GiftCardQueue ref="queue" />
    </main>
  </div>
</template>

<script setup>
import GiftCardQueue from './components/GiftCardQueue.vue';
</script>