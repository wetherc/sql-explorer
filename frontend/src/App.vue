<script setup lang="ts">
import { useConnectionStore } from '@/stores/connection'
import ConnectionView from '@/views/ConnectionView.vue'
import QueryView from '@/views/QueryView.vue'
import DbExplorer from '@/components/DbExplorer.vue' // Placeholder for now

const connectionStore = useConnectionStore()
</script>

<template>
  <div class="app-container">
    <aside class="sidebar">
      <!-- Database Explorer will go here -->
      <DbExplorer v-if="connectionStore.isConnected" />
      <div v-else class="sidebar-placeholder">
        Connect to a database to see the explorer.
      </div>
    </aside>
    <main class="content-area">
      <ConnectionView v-if="!connectionStore.isConnected" />
      <QueryView v-else />
    </main>
  </div>
</template>

<style>
/* Basic reset and global styles */
body {
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
  background-color: #e9ecef;
  color: #212529;
  height: 100vh; /* Ensure body takes full viewport height */
  overflow: hidden; /* Prevent scrollbars on body */
}

#app {
  height: 100%; /* Ensure #app takes full height */
}

.app-container {
  display: flex;
  height: 100vh; /* Full viewport height */
  overflow: hidden;
}

.sidebar {
  width: 250px; /* Initial width for the sidebar */
  background-color: #f8f9fa;
  border-right: 1px solid #dee2e6;
  padding: 10px;
  overflow-y: auto; /* Allow scrolling if explorer content is tall */
  flex-shrink: 0; /* Prevent sidebar from shrinking */
}

.sidebar-placeholder {
  color: #6c757d;
  text-align: center;
  padding-top: 20px;
}

.content-area {
  flex-grow: 1; /* Main content area takes remaining space */
  display: flex;
  flex-direction: column;
  overflow: hidden; /* Ensure content doesn't overflow */
}
</style>
