<script setup lang="ts">
import { useConnectionStore } from '@/stores/connection'
import ConnectionView from '@/views/ConnectionView.vue'
import QueryView from '@/views/QueryView.vue'

const connectionStore = useConnectionStore()
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
