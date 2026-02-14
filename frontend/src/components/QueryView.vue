<template>
  <div class="query-view d-flex flex-column">
    <div class="editor-toolbar d-flex align-center">
      <v-btn @click="handleExecute" :loading="queryState.loading" class="mr-4">Execute</v-btn>
      <v-select
        v-model="currentConnectionId"
        :items="activeConnectionItems"
        label="Connection"
        density="compact"
        hide-details
        style="max-width: 300px"
      ></v-select>
    </div>
    <splitpanes horizontal style="height: 100%" @resize="handlePaneResize">
      <pane size="40">
        <MonacoEditor
          v-model="query"
          language="sql"
          theme="vs-dark"
          style="height: 100%;"
          @mount="handleEditorMount"
        />
      </pane>
      <pane size="60">
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
      </pane>
    </splitpanes>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onUnmounted, watch } from 'vue'
import MonacoEditor from 'monaco-editor-vue3'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import { useQueryStore } from '@/stores/query'
import { useTabsStore, type QueryTab } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'

const props = defineProps<{
  initialQuery: string
  tabId: string
}>()

const queryStore = useQueryStore()
const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()

const query = ref(props.initialQuery)
const activeResultsTab = ref('result-0')
const monacoInstance = ref(null) // Renamed from monacoEditorRef

const currentTab = computed(() => tabsStore.tabs.find((t: QueryTab) => t.id === props.tabId))

// Get a reactive reference to the state for this specific tab
const queryState = computed(() => queryStore.getStateForTab(props.tabId))

const activeConnectionItems = computed(() =>
  Object.values(connectionStore.activeConnections).map(conn => ({
    title: conn.name,
    value: conn.id,
  }))
)

const currentConnectionId = computed({
  get: () => currentTab.value?.connectionId ?? null,
  set: (newConnectionId) => {
    if (newConnectionId && currentTab.value) {
      tabsStore.updateTabConnection(props.tabId, newConnectionId)
    }
  }
})

function handleExecute() {
  if (currentTab.value) {
    queryStore.executeQuery(props.tabId, currentTab.value.connectionId, query.value)
    // Also update the query in the tabs store so it's saved
    currentTab.value.query = query.value
  }
}

function handleEditorMount(editor: any) {
  monacoInstance.value = editor
  console.log('Monaco editor mounted and instance stored.')
}

function handlePaneResize() {
  if (monacoInstance.value) {
    monacoInstance.value.layout()
    console.log('Monaco editor layout() called via instance.')
  } else {
    console.log('Monaco editor instance not yet available for layout.')
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
.results-pane {
  height: 100%;
  display: flex;
  flex-direction: column;
}
.v-window {
  flex-grow: 1;
  overflow-y: auto;
}

:deep(.splitpanes__splitter) {
  background-color: #333;
  border: 1px solid #444;
}
</style>
