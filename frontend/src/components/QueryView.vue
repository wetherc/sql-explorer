<template>
  <div class="query-view d-flex flex-column">
    <div class="editor-toolbar">
      <v-btn @click="handleExecute" :loading="queryState.loading">Execute</v-btn>
    </div>
    <div class="editor-pane">
      <MonacoEditor
        v-model="query"
        language="sql"
        theme="vs-dark"
        style="height: 100%;"
      />
    </div>
    <div class="results-pane">
      <v-tabs v-model="activeResultsTab" bg-color="secondary">
        <v-tab v-for="(_, index) in queryState.results" :key="index" :value="`result-${index}`">
          Result {{ index + 1 }}
        </v-tab>
        <v-tab value="messages">Messages</v-tab>
      </v-tabs>

      <v-window v-model="activeResultsTab">
        <v-window-item v-for="(result, index) in queryState.results" :key="index" :value="`result-${index}`">
          <v-data-table
            :headers="result.columns"
            :items="result.rows"
            :loading="queryState.loading"
            density="compact"
            class="fill-height"
          ></v-data-table>
        </v-window-item>
        <v-window-item value="messages">
          <v-list density="compact">
            <v-list-item v-for="(msg, i) in queryState.messages" :key="i" :title="msg"></v-list-item>
          </v-list>
          <div v-if="queryState.error" class="pa-4 text-error">
            {{ queryState.error }}
          </div>
        </v-window-item>
      </v-window>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onUnmounted } from 'vue'
import MonacoEditor from 'monaco-editor-vue3'
import { useQueryStore } from '@/stores/query'
import { useTabsStore, type QueryTab } from '@/stores/tabs'

const props = defineProps<{
  initialQuery: string
  tabId: string
}>()

const queryStore = useQueryStore()
const tabsStore = useTabsStore()

const query = ref(props.initialQuery)
const activeResultsTab = ref('result-0')

// Get a reactive reference to the state for this specific tab
const queryState = computed(() => queryStore.getStateForTab(props.tabId))

function handleExecute() {
  queryStore.executeQuery(props.tabId, query.value)
  // Also update the query in the tabs store so it's saved
  const currentTab = tabsStore.tabs.find((t: QueryTab) => t.id === props.tabId)
  if (currentTab) {
    currentTab.query = query.value
  }
}

// Clean up the query state when the tab is closed
onUnmounted(() => {
  queryStore.clearStateForTab(props.tabId)
})
</script>

<style scoped>
.query-view {
  height: calc(100vh - 112px); /* Adjust based on app bar and tabs height */
}
.editor-toolbar {
  padding: 8px;
  border-bottom: 1px solid #444;
}
.editor-pane {
  height: 40%;
  min-height: 100px;
}
.results-pane {
  height: 60%;
  border-top: 1px solid #ccc;
  display: flex;
  flex-direction: column;
}
.v-window {
  flex-grow: 1;
  overflow-y: auto;
}
</style>
