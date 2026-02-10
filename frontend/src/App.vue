<script setup lang="ts">
import { useConnectionStore } from '@/stores/connection'
import ConnectionView from '@/views/ConnectionView.vue'
import QueryTabs from '@/components/QueryTabs.vue'
import DbExplorer from '@/components/DbExplorer.vue'
import Splitter from 'primevue/splitter'
import SplitterPanel from 'primevue/splitterpanel'
import Toast from 'primevue/toast'
import ProgressBar from 'primevue/progressbar'

const connectionStore = useConnectionStore()
</script>

<template>
  <div class="app-container">
    <ProgressBar v-if="connectionStore.isConnecting" mode="indeterminate" style="height: .5em" class="fixed top-0 left-0 w-full" />
    <Splitter style="height: 100vh">
      <SplitterPanel :size="20" :min-size="10">
        <div class="sidebar">
          <DbExplorer v-if="connectionStore.isConnected" />
          <div v-else class="sidebar-placeholder">
            Connect to a database to see the explorer.
          </div>
        </div>
      </SplitterPanel>
      <SplitterPanel :size="80">
        <main class="content-area">
          <ConnectionView v-if="!connectionStore.isConnected" />
          <QueryTabs v-else />
        </main>
      </SplitterPanel>
    </Splitter>
    <Toast />
  </div>
</template>

<style>
/* Basic reset and global styles */
body {
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
  height: 100vh;
  overflow: hidden;
}

#app {
  height: 100%;
}

.sidebar, .content-area {
  width: 100%;
  height: 100%;
  overflow: auto;
  padding: 1rem;
}

.sidebar-placeholder {
  color: var(--p-text-color);
  text-align: center;
  padding-top: 20px;
}
</style>
