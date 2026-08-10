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

      <v-tooltip location="bottom" text="Give the values of the named parameters">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            size="small"
            prepend-icon="mdi-variable"
            text="Parameters"
            data-test="parameters-button"
            @click="editParams()"
          />
        </template>
      </v-tooltip>

      <v-menu v-if="supportsExplain">
        <template #activator="{ props: menu }">
          <v-btn
            v-bind="menu"
            :disabled="!canRun"
            size="small"
            prepend-icon="mdi-sitemap-outline"
            text="Plan"
            data-test="plan-button"
          />
        </template>
        <v-list density="compact">
          <v-list-item data-test="plan-estimated" @click="readPlan(PlanKind.Estimated)">
            <v-list-item-title>Estimated plan</v-list-item-title>
            <v-list-item-subtitle>The statement does not run.</v-list-item-subtitle>
          </v-list-item>
          <v-list-item data-test="plan-actual" @click="askForActualPlan()">
            <v-list-item-title>Actual plan</v-list-item-title>
            <v-list-item-subtitle>The statement runs.</v-list-item-subtitle>
          </v-list-item>
        </v-list>
      </v-menu>

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
          @format-failed="onFormatFailed"
          @show-keys="ui.setKeyboardHelpOpen(true)"
        />
      </pane>
      <pane :size="100 - editorSize" min-size="15">
        <div class="results-pane">
          <v-tabs
            :model-value="state.activePaneId ?? MESSAGES_TAB"
            density="compact"
            class="results-tabs"
            @update:model-value="onResultTabChange"
          >
            <v-tab
              v-for="pane in state.panes"
              :key="pane.id"
              :value="pane.id"
              data-test="result-tab"
            >
              <v-icon
                size="x-small"
                class="mr-1"
                :color="pane.pinned ? 'warning' : undefined"
                :aria-label="pane.pinned ? 'Let this result go' : 'Keep this result'"
                data-test="pin-result"
                @click.stop="queries.togglePin(tab.id, pane.id)"
              >
                {{ pane.pinned ? 'mdi-pin' : 'mdi-pin-outline' }}
              </v-icon>
              <span>{{ paneLabel(pane) }}</span>
              <v-icon
                v-if="pane.pinned"
                size="x-small"
                class="ml-2"
                aria-label="Close this result"
                data-test="close-result"
                @click.stop="queries.closePane(tab.id, pane.id)"
              >
                mdi-close
              </v-icon>
            </v-tab>
            <v-tab :value="MESSAGES_TAB" data-test="messages-tab">
              Messages
              <v-badge v-if="state.error" color="error" dot inline />
            </v-tab>
          </v-tabs>

          <div class="results-body">
            <template v-for="pane in state.panes" :key="pane.id">
              <ResultsGrid
                v-if="state.activePaneId === pane.id"
                :result="pane.result"
                @export="onExport"
                @export-all="onExportAll"
                @copied="onCopied"
              />
            </template>

            <div v-if="state.activePaneId === null" class="messages pa-3">
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
                :class="`message-${message.level}`"
                data-test="query-message"
              >
                <v-icon v-if="message.level !== 'info'" size="14" class="mr-1">
                  {{ message.level === 'error' ? 'mdi-alert-circle' : 'mdi-alert' }}
                </v-icon>
                {{ message.text }}
                <span v-if="message.detail" class="message-detail">{{ message.detail }}</span>
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

    <v-dialog v-model="askingTable" max-width="420">
      <v-card>
        <v-card-title class="text-subtitle-1">Name the table</v-card-title>
        <v-card-text>
          <v-text-field
            v-model="insertTable"
            label="Table"
            hint="The name the INSERT statements write to."
            persistent-hint
            autofocus
            data-test="insert-table-name"
          />
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Cancel" @click="askingTable = false" />
          <v-btn
            color="primary"
            text="Export"
            data-test="insert-table-confirm"
            @click="confirmInsertExport"
          />
        </v-card-actions>
      </v-card>
    </v-dialog>

    <v-dialog v-model="askingParams" max-width="520">
      <v-card>
        <v-card-title class="text-subtitle-1">Give the values of the parameters</v-card-title>
        <v-card-text class="d-flex flex-column ga-3">
          <div
            v-for="row of paramRows"
            :key="row.name"
            class="d-flex align-center ga-2"
            data-test="parameter-row"
          >
            <div class="param-name text-medium-emphasis">:{{ row.name }}</div>
            <v-select
              v-model="row.kind"
              :items="PARAM_KINDS"
              item-title="title"
              item-value="value"
              label="Form"
              density="compact"
              hide-details
              class="param-kind"
              :data-test="`parameter-kind-${row.name}`"
            />
            <v-text-field
              v-model="row.text"
              :disabled="row.kind === ParamKind.Null"
              label="Value"
              density="compact"
              hide-details
              :data-test="`parameter-value-${row.name}`"
            />
          </div>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Cancel" data-test="parameters-cancel" @click="cancelParams" />
          <v-btn
            color="primary"
            text="Use these values"
            data-test="parameters-confirm"
            @click="confirmParams"
          />
        </v-card-actions>
      </v-card>
    </v-dialog>

    <v-dialog v-model="askingPlan" max-width="460">
      <v-card>
        <v-card-title class="text-subtitle-1">Run the statement for its plan</v-card-title>
        <v-card-text>
          The actual plan comes from a real run. The statement runs on the server, so a statement
          that writes rows writes them, and a statement on Athena scans data.
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Cancel" @click="askingPlan = false" />
          <v-btn
            color="primary"
            text="Run it"
            data-test="plan-actual-confirm"
            @click="confirmActualPlan"
          />
        </v-card-actions>
      </v-card>
    </v-dialog>

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
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog'
import SqlEditor from './SqlEditor.vue'
import ResultsGrid from './ResultsGrid.vue'
import { api } from '@/lib/api'
import { forgetTabActions, registerTabActions } from '@/lib/commands'
import { exportFileName, toCsv, toInsertStatements, toJson, toMarkdown } from '@/lib/export'
import { bytesToBase64, toXlsx } from '@/lib/xlsx'
import { formatClockTime, formatRowCount } from '@/lib/format'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useHistoryStore } from '@/stores/history'
import { useQueryStore } from '@/stores/query'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import { alignParams, needsAValue, paramsForRun } from '@/lib/params'
import { Dialect, ParamKind, PlanKind, type ParamValue, type ResultSet } from '@/types/api'
import type { ExportFormat } from './ResultsGrid.vue'
import type { ResultPane } from '@/stores/query'
import type { QueryTab } from '@/stores/tabs'

/** The value that stands for the Messages tab. */
const MESSAGES_TAB = 'messages'

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
const savingQuery = ref(false)
const saveName = ref('')
const saveFolder = ref('')
const askingTable = ref(false)
const askingPlan = ref(false)
const askingParams = ref(false)
/** The rows of the parameter dialog, which the user edits before a run. */
const paramRows = ref<ParamValue[]>([])
/** The run that waits for the values of the parameters. */
let pendingRun: ((values: Record<string, unknown>) => void) | null = null
const insertTable = ref('')
/** The rows that wait while the user names the table for the INSERT form. */
let pendingInsert: ResultSet | null = null

const state = computed(() => queries.stateFor(props.tab.id))

/**
 * The connections the tab can run on, and the one it names when that one is
 * not open. A tab that the workspace held can name a connection that is
 * closed, or one that the user has deleted, and the select would then show
 * the identifier of that connection to the reader.
 */
const connectionItems = computed(() => {
  const items = connections.activeList.map((connection) => ({
    title: connection.name,
    value: connection.id,
  }))
  const id = props.tab.connectionId
  if (id && !items.some((item) => item.value === id)) {
    items.unshift({ title: `${connections.nameFor(id)} (not open)`, value: id })
  }
  return items
})

const dialect = computed<Dialect>(() => {
  const id = props.tab.connectionId
  return id ? (connections.active[id]?.dialect ?? Dialect.MsSql) : Dialect.MsSql
})

const supportsExplain = computed(() => {
  const id = props.tab.connectionId
  return id !== null && (connections.active[id]?.capabilities.supportsExplain ?? false)
})

const canRun = computed(() => {
  const id = props.tab.connectionId
  return id !== null && connections.isActive(id)
})

/**
 * Names a result. A result the user keeps also carries the time of its run,
 * so that two results of the same statement can be told apart.
 */
function paneLabel(pane: ResultPane): string {
  const name = pane.label ?? `Result ${pane.number}`
  const head = `${name} (${formatRowCount(pane.result.rows.length)})`
  return pane.pinned ? `${head} at ${formatClockTime(pane.ranAt)}` : head
}

/** The forms a value can take in the parameter dialog. */
const PARAM_KINDS = [
  { title: 'Text', value: ParamKind.Text },
  { title: 'Number', value: ParamKind.Number },
  { title: 'True or false', value: ParamKind.Boolean },
  { title: 'Empty value', value: ParamKind.Null },
]

/**
 * Reads the names of the parameters of a statement and hands the values to
 * the caller. A name that has no value opens the dialog, and the caller runs
 * once the user has given the values.
 */
async function withParams(
  statement: string,
  action: (values?: Record<string, unknown>) => void,
): Promise<void> {
  let names: string[] = []
  try {
    names = await api.queryParameters(statement, dialect.value)
  } catch (error) {
    ui.reportError(error)
    return
  }
  if (names.length === 0) {
    action(undefined)
    return
  }

  const rows = alignParams(names, props.tab.params)
  tabs.setParams(props.tab.id, rows)
  if (rows.some(needsAValue)) {
    paramRows.value = rows.map((row) => ({ ...row }))
    pendingRun = (values) => action(values)
    askingParams.value = true
    return
  }
  action(paramsForRun(rows))
}

/** Opens the parameter dialog on its own, so a value can be changed. */
async function editParams(): Promise<void> {
  let names: string[] = []
  try {
    names = await api.queryParameters(props.tab.query, dialect.value)
  } catch (error) {
    ui.reportError(error)
    return
  }
  if (names.length === 0) {
    ui.warn('This statement holds no parameter.')
    return
  }
  paramRows.value = alignParams(names, props.tab.params)
  pendingRun = null
  askingParams.value = true
}

function confirmParams(): void {
  const rows = paramRows.value.map((row) => ({ ...row }))
  tabs.setParams(props.tab.id, rows)
  askingParams.value = false
  const next = pendingRun
  pendingRun = null
  next?.(paramsForRun(rows))
}

function cancelParams(): void {
  askingParams.value = false
  pendingRun = null
}

function onResultTabChange(value: unknown): void {
  const id = String(value)
  queries.selectPane(props.tab.id, id === MESSAGES_TAB ? null : id)
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
  await withParams(statement, (values) => {
    void queries.execute(props.tab.id, connectionId, statement, values)
  })
}

function runStatement(statement?: string): void {
  const text = statement ?? editorRef.value?.currentStatement() ?? props.tab.query
  void run(text)
}

function runAll(): void {
  void run(props.tab.query)
}

/** Reads the plan of the statement under the cursor. */
function readPlan(kind: PlanKind): void {
  const connectionId = props.tab.connectionId
  if (!connectionId) {
    ui.warn('Choose a connection before you read a plan.')
    return
  }
  const text = editorRef.value?.currentStatement() ?? props.tab.query
  void withParams(text, (values) => {
    void queries.explain(props.tab.id, connectionId, text, kind, values)
  })
}

function askForActualPlan(): void {
  askingPlan.value = true
}

function confirmActualPlan(): void {
  askingPlan.value = false
  readPlan(PlanKind.Actual)
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

/** The name and the extension of the file each form of export writes. */
const EXPORT_FILES: Record<ExportFormat, { label: string; extension: string }> = {
  csv: { label: 'CSV', extension: 'csv' },
  json: { label: 'JSON', extension: 'json' },
  markdown: { label: 'Markdown', extension: 'md' },
  insert: { label: 'SQL', extension: 'sql' },
  xlsx: { label: 'Excel', extension: 'xlsx' },
}

function onExport(format: ExportFormat, rows: ResultSet): void {
  if (format === 'insert') {
    pendingInsert = rows
    insertTable.value = props.tab.title
    askingTable.value = true
    return
  }
  void exportResult(rows, format)
}

function confirmInsertExport(): void {
  const rows = pendingInsert
  askingTable.value = false
  pendingInsert = null
  if (rows) {
    void exportResult(rows, 'insert')
  }
}

/**
 * Writes every row of the statement to a file. The backend runs the
 * statement again with a higher row limit and writes the file itself, so a
 * large result never passes through the interface.
 */
async function onExportAll(format: 'csv' | 'json'): Promise<void> {
  const connectionId = props.tab.connectionId
  if (!connectionId) {
    return
  }
  try {
    const path = await saveFileDialog({
      defaultPath: exportFileName(props.tab.title, format),
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    })
    if (!path) {
      return
    }
    const summary = await api.exportQuery({
      connectionId,
      requestId: `export-${props.tab.id}-${Date.now()}`,
      query: props.tab.query.trim(),
      path,
      format,
      maxRows: settings.settings.exportRowLimit,
      queryParams: paramsForRun(props.tab.params),
    })
    if (summary.truncated) {
      ui.warn(
        `The export limit stopped the read at ${summary.rows.toLocaleString()} rows.`,
        'Raise the export limit in the settings.',
      )
    } else {
      ui.success(`${summary.rows.toLocaleString()} rows are written to ${path}.`)
    }
  } catch (error) {
    ui.reportError(error)
  }
}

async function exportResult(result: ResultSet, format: ExportFormat): Promise<void> {
  const file = EXPORT_FILES[format]
  try {
    const path = await saveFileDialog({
      defaultPath: exportFileName(props.tab.title, file.extension),
      filters: [{ name: file.label, extensions: [file.extension] }],
    })
    if (!path) {
      return
    }
    if (format === 'xlsx') {
      await api.writeBinaryFile(path, bytesToBase64(toXlsx(result, props.tab.title)))
    } else {
      await api.writeTextFile(path, textFor(result, format))
    }
    ui.success(`The result is written to ${path}.`)
  } catch (error) {
    ui.reportError(error)
  }
}

/** Writes the result in the text form the user asked for. */
function textFor(result: ResultSet, format: ExportFormat): string {
  if (format === 'json') {
    return toJson(result)
  }
  if (format === 'markdown') {
    return toMarkdown(result)
  }
  if (format === 'insert') {
    return toInsertStatements(result, insertTable.value.trim() || 'the_table', dialect.value)
  }
  return toCsv(result)
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

// The shell holds the keys, and the editor of this tab holds the text, so
// the view of each tab records what it can do under its own identifier.
onMounted(() => {
  registerTabActions(props.tab.id, {
    runStatement: () => runStatement(),
    runAll,
    cancel,
    format: formatStatement,
  })
})

onBeforeUnmount(() => {
  forgetTabActions(props.tab.id)
})

defineExpose({ runStatement, runAll, formatStatement, readPlan })
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

.param-name {
  min-width: 90px;
  font-family: ui-monospace, monospace;
}

.param-kind {
  max-width: 150px;
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

.message-warning {
  color: rgb(var(--v-theme-warning));
}

.message-error {
  color: rgb(var(--v-theme-error));
}

.message-detail {
  margin-left: 0.5rem;
  opacity: 0.7;
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
