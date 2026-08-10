<template>
  <div class="query-view">
    <div class="toolbar d-flex align-center ga-2 px-2 py-1">
      <v-tooltip location="bottom" text="Run the statement under the cursor (Ctrl/Cmd + Enter)">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            :disabled="!canRun"
            :loading="state.running"
            color="primary"
            variant="flat"
            size="small"
            prepend-icon="mdi-play"
            text="Run"
            data-test="run-button"
            @click="runStatement()"
          />
        </template>
      </v-tooltip>

      <v-tooltip location="bottom" text="Run the whole script (Ctrl/Cmd + Shift + Enter)">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            :disabled="!canRun"
            size="small"
            prepend-icon="mdi-playlist-play"
            text="Run all"
            data-test="run-all-button"
            @click="runAll()"
          />
        </template>
      </v-tooltip>

      <v-btn
        v-if="state.running"
        color="error"
        size="small"
        prepend-icon="mdi-stop"
        text="Stop"
        data-test="cancel-button"
        @click="cancel()"
      />

      <v-divider vertical class="mx-1" />

      <v-tooltip location="bottom" text="Lay out the statement (Shift + Alt + F)">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            size="small"
            prepend-icon="mdi-format-align-left"
            text="Format"
            data-test="format-button"
            @click="formatStatement()"
          />
        </template>
      </v-tooltip>

      <v-select
        :model-value="tab.connectionId"
        :items="connectionItems"
        item-title="title"
        item-value="value"
        label="Connection"
        density="compact"
        hide-details
        class="connection-select"
        data-test="connection-select"
        @update:model-value="onConnectionChange"
      />

      <v-spacer />

      <v-btn
        size="small"
        prepend-icon="mdi-content-save-outline"
        text="Save"
        data-test="save-query-button"
        @click="savingQuery = true"
      />
    </div>

    <splitpanes horizontal class="panes" @resize="onPaneResize">
      <pane :size="editorSize" min-size="15">
        <SqlEditor
          ref="editorRef"
          :model-value="tab.query"
          :theme="settings.editorTheme"
          :font-size="settings.settings.fontSize"
          :word-wrap="settings.settings.wordWrap"
          :show-line-numbers="settings.settings.showLineNumbers"
          :schema-index="explorer.schemaIndex"
          :dialect="dialect"
          @update:model-value="onQueryChange"
          @execute="runStatement"
          @execute-all="runAll"
          @format-failed="onFormatFailed"
        />
      </pane>
      <pane :size="100 - editorSize" min-size="15">
        <div class="results-pane">
          <v-tabs v-model="activeResultTab" density="compact" class="results-tabs">
            <v-tab
              v-for="(result, index) in state.results"
              :key="index"
              :value="`result-${index}`"
              :text="resultLabel(result, index)"
              data-test="result-tab"
            />
            <v-tab value="messages" data-test="messages-tab">
              Messages
              <v-badge v-if="state.error" color="error" dot inline />
            </v-tab>
          </v-tabs>

          <div class="results-body">
            <template v-for="(result, index) in state.results" :key="index">
              <ResultsGrid
                v-if="activeResultTab === `result-${index}`"
                :result="result"
                @export="(format) => exportResult(result, format)"
                @copied="onCopied"
              />
            </template>

            <div v-if="activeResultTab === 'messages'" class="messages pa-3">
              <v-alert
                v-if="state.error"
                type="error"
                variant="tonal"
                density="compact"
                class="mb-3"
                data-test="query-error"
              >
                <div class="font-weight-medium">{{ state.error.message }}</div>
                <pre v-if="state.error.detail" class="error-detail">{{ state.error.detail }}</pre>
              </v-alert>

              <div
                v-for="(message, index) in state.messages"
                :key="index"
                class="message-line"
                data-test="query-message"
              >
                {{ message }}
              </div>

              <div
                v-if="!state.error && state.messages.length === 0"
                class="text-medium-emphasis"
                data-test="no-messages"
              >
                Run a statement to see its messages here.
              </div>
            </div>
          </div>
        </div>
      </pane>
    </splitpanes>

    <v-dialog v-model="savingQuery" max-width="480">
      <v-card>
        <v-card-title class="text-subtitle-1">Save this statement</v-card-title>
        <v-card-text class="d-flex flex-column ga-3">
          <v-text-field v-model="saveName" label="Name" autofocus data-test="save-query-name" />
          <v-text-field v-model="saveFolder" label="Folder" placeholder="Saved queries" />
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Cancel" @click="savingQuery = false" />
          <v-btn color="primary" text="Save" data-test="save-query-confirm" @click="confirmSave" />
        </v-card-actions>
      </v-card>
    </v-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog'
import SqlEditor from './SqlEditor.vue'
import ResultsGrid from './ResultsGrid.vue'
import { api } from '@/lib/api'
import { exportFileName, toCsv, toJson } from '@/lib/export'
import { formatRowCount } from '@/lib/format'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useHistoryStore } from '@/stores/history'
import { useQueryStore } from '@/stores/query'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import { Dialect, type ResultSet } from '@/types/api'
import type { QueryTab } from '@/stores/tabs'

const props = defineProps<{ tab: QueryTab }>()

const tabs = useTabsStore()
const queries = useQueryStore()
const connections = useConnectionsStore()
const explorer = useExplorerStore()
const history = useHistoryStore()
const settings = useSettingsStore()
const ui = useUiStore()

const editorRef = ref<InstanceType<typeof SqlEditor> | null>(null)
const editorSize = ref(45)
const activeResultTab = ref('messages')
const savingQuery = ref(false)
const saveName = ref('')
const saveFolder = ref('')

const state = computed(() => queries.stateFor(props.tab.id))

const connectionItems = computed(() =>
  connections.activeList.map((connection) => ({
    title: connection.name,
    value: connection.id,
  })),
)

const dialect = computed<Dialect>(() => {
  const id = props.tab.connectionId
  return id ? (connections.active[id]?.dialect ?? Dialect.MsSql) : Dialect.MsSql
})

const canRun = computed(() => {
  const id = props.tab.connectionId
  return id !== null && connections.isActive(id)
})

function resultLabel(result: ResultSet, index: number): string {
  return `Result ${index + 1} (${formatRowCount(result.rows.length)})`
}

function onQueryChange(value: string): void {
  tabs.setQuery(props.tab.id, value)
}

function onConnectionChange(value: string | null): void {
  tabs.setConnection(props.tab.id, value)
}

function onPaneResize(panes: Array<{ size: number }>): void {
  if (panes[0]) {
    editorSize.value = panes[0].size
  }
}

function onCopied(): void {
  ui.success('The rows are on the clipboard.')
}

async function run(statement: string): Promise<void> {
  const connectionId = props.tab.connectionId
  if (!connectionId) {
    ui.warn('Choose a connection before you run a statement.')
    return
  }
  const succeeded = await queries.execute(props.tab.id, connectionId, statement)
  const current = queries.stateFor(props.tab.id)
  activeResultTab.value = succeeded && current.results.length > 0 ? 'result-0' : 'messages'
}

function runStatement(statement?: string): void {
  const text = statement ?? editorRef.value?.currentStatement() ?? props.tab.query
  void run(text)
}

function runAll(): void {
  void run(props.tab.query)
}

function formatStatement(): void {
  editorRef.value?.format()
}

function onFormatFailed(message: string): void {
  ui.warn('The statement could not be laid out.', message)
}

function cancel(): void {
  const connectionId = props.tab.connectionId
  if (connectionId) {
    void queries.cancel(props.tab.id, connectionId)
  }
}

async function exportResult(result: ResultSet, format: 'csv' | 'json'): Promise<void> {
  const suggested = exportFileName(props.tab.title, format)
  try {
    const path = await saveFileDialog({
      defaultPath: suggested,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    })
    if (!path) {
      return
    }
    const contents = format === 'csv' ? toCsv(result) : toJson(result)
    await api.writeTextFile(path, contents)
    ui.success(`The result is written to ${path}.`)
  } catch (error) {
    ui.reportError(error)
  }
}

async function confirmSave(): Promise<void> {
  const saved = await history.save({
    id: props.tab.savedQueryId ?? undefined,
    name: saveName.value,
    query: props.tab.query,
    connectionId: props.tab.connectionId,
    folder: saveFolder.value,
  })
  if (saved) {
    tabs.rename(props.tab.id, saved.name)
    tabs.markClean(props.tab.id)
    savingQuery.value = false
  }
}

watch(savingQuery, (open) => {
  if (open) {
    saveName.value = props.tab.title
  }
})

// A new result set moves the view to the first result.
watch(
  () => state.value.results.length,
  (count) => {
    if (count > 0 && activeResultTab.value === 'messages') {
      activeResultTab.value = 'result-0'
    }
  },
)

defineExpose({ runStatement, runAll, formatStatement })
</script>

<style scoped>
.query-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.toolbar {
  flex: 0 0 auto;
  border-bottom: 1px solid rgb(var(--v-theme-surface-variant));
  background: rgb(var(--v-theme-surface));
}

.connection-select {
  max-width: 260px;
}

.panes {
  flex: 1 1 auto;
  min-height: 0;
}

.results-pane {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: rgb(var(--v-theme-surface));
}

.results-tabs {
  flex: 0 0 auto;
  border-bottom: 1px solid rgb(var(--v-theme-surface-variant));
}

.results-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.messages {
  overflow: auto;
  font-size: 0.8125rem;
}

.message-line {
  padding: 2px 0;
  font-family: ui-monospace, monospace;
}

.error-detail {
  margin-top: 6px;
  white-space: pre-wrap;
  font-size: 0.75rem;
  opacity: 0.85;
}

:deep(.splitpanes__splitter) {
  background: rgb(var(--v-theme-surface-variant));
  min-height: 4px;
}
</style>
