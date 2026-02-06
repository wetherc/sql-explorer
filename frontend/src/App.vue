<script setup lang="ts">
import { useConnectionStore } from '@/stores/connection'
import ConnectionView from '@/views/ConnectionView.vue'

const connectionStore = useConnectionStore()

// A placeholder for the main query view
const QueryView = {
  template: `
    <div>
      <h1>Query View</h1>
      <p>You are connected!</p>
      <button @click="connectionStore.disconnect()">Disconnect</button>
    </div>
  `,
  setup() {
    const connectionStore = useConnectionStore()
    return { connectionStore }
  }
}
</script>

<template>
  <main>
    <ConnectionView v-if="!connectionStore.isConnected" />
    <QueryView v-else />
  </main>
</template>

<style>
/* Basic reset and global styles */
body {
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
  background-color: #e9ecef;
  color: #212529;
}
</style>
