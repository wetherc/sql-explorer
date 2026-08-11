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

      <!-- Stop stands beside Run, so it carries a weight of its own. It is
           quieter than Run, because Run is the button of the work. -->
      <v-btn
        v-if="state.running"
        color="error"
        variant="tonal"
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
        :loading="savingFile"
        data-test="save-file-button"
        @click="saveToFile"
      />

      <v-btn
        size="small"
        variant="text"
        icon="mdi-bookmark-outline"
        aria-label="Save this statement in the library"
        title="Save this statement in the library"
        data-test="save-query-button"
        @click="savingQuery = true"
      />
    </div>

    <!-- The bar names the parameters that the statement holds, so the
         feature is plain to see and the values are one click away. -->
    <div
      v-if="paramNames.length > 0"
      class="parameter-bar d-flex align-center flex-wrap ga-1 px-2 py-1"
      data-test="parameter-bar"
    >
      <v-chip
        v-for="name of paramNames"
        :key="name"
        size="x-small"
        label
        :color="paramIsUnset(name) ? 'warning' : undefined"
        :data-test="`parameter-chip-${name}`"
        @click="editParams()"
      >
        {{ paramChipLabel(name, tab.params) }}
      </v-chip>
    </div>

    <splitpanes
      :horizontal="resultsBelow"
      class="panes"
      :class="{ 'results-away': resultsCollapsed }"
      @resize="onPaneResize"
    >
      <pane
        :size="resultsCollapsed ? 100 : editorSize"
        :min-size="resultsCollapsed ? 100 : MIN_EDITOR_SIZE"
      >
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
      <!-- The pane keeps its content while the panel is away, so the grid
           holds its scroll place, its filter and its selection. -->
      <pane
        :size="resultsCollapsed ? 0 : 100 - editorSize"
        :min-size="resultsCollapsed ? 0 : MIN_EDITOR_SIZE"
      >
        <div v-show="!resultsCollapsed" class="results-pane">
          <!-- The actions of a result sit beside the strip and act on the
               result that is open. A tab is itself a button, so a button
               inside it would nest one inside another, and the two would
               fight for the same click. -->
          <div class="results-tab-row d-flex align-center">
            <v-tabs
              :model-value="state.activePaneId ?? MESSAGES_TAB"
              show-arrows
              class="results-tabs flex-grow-1"
              @update:model-value="onResultTabChange"
            >
              <v-tab
                v-for="pane in state.panes"
                :key="pane.id"
                :value="pane.id"
                data-test="result-tab"
              >
                <v-icon v-if="pane.pinned" size="x-small" class="mr-1" aria-hidden="true">
                  mdi-pin
                </v-icon>
                <span>{{ paneLabel(pane) }}</span>
                <span v-if="pane.pinned" class="app-visually-hidden">, kept</span>
              </v-tab>
              <v-tab :value="MESSAGES_TAB" data-test="messages-tab">
                Messages
                <v-badge v-if="state.error" color="error" dot inline />
              </v-tab>
            </v-tabs>

            <div class="results-actions d-flex align-center ga-1 px-1">
              <template v-if="activePane">
                <v-tooltip
                  location="bottom"
                  :text="activePane.pinned ? 'Let this result go' : 'Keep this result'"
                >
                  <template #activator="{ props: tip }">
                    <v-btn
                      v-bind="tip"
                      :icon="activePane.pinned ? 'mdi-pin' : 'mdi-pin-outline'"
                      :color="activePane.pinned ? 'warning' : undefined"
                      size="small"
                      :aria-label="activePane.pinned ? 'Let this result go' : 'Keep this result'"
                      data-test="pin-result"
                      @click="queries.togglePin(tab.id, activePane.id)"
                    />
                  </template>
                </v-tooltip>
                <v-tooltip location="bottom" text="Close this result">
                  <template #activator="{ props: tip }">
                    <v-btn
                      v-bind="tip"
                      icon="mdi-close"
                      size="small"
                      aria-label="Close this result"
                      data-test="close-result"
                      @click="queries.closePane(tab.id, activePane.id)"
                    />
                  </template>
                </v-tooltip>
              </template>

              <v-tooltip
                location="bottom"
                :text="
                  resultsBelow
                    ? 'Move the results beside the editor'
                    : 'Move the results below the editor'
                "
              >
                <template #activator="{ props: tip }">
                  <v-btn
                    v-bind="tip"
                    :icon="resultsBelow ? 'mdi-dock-right' : 'mdi-dock-bottom'"
                    size="small"
                    :aria-label="
                      resultsBelow
                        ? 'Move the results beside the editor'
                        : 'Move the results below the editor'
                    "
                    data-test="move-results"
                    @click="layout.toggleResultsOrientation()"
                  />
                </template>
              </v-tooltip>

              <v-tooltip location="bottom" text="Put the results away">
                <template #activator="{ props: tip }">
                  <v-btn
                    v-bind="tip"
                    icon="mdi-chevron-down"
                    size="small"
                    aria-label="Put the results away"
                    data-test="collapse-results"
                    @click="layout.setResultsCollapsed(true)"
                  />
                </template>
              </v-tooltip>
            </div>
          </div>

          <div class="results-body">
            <!-- The grids of the results the user opened last stay mounted
                 behind v-show, so the filter, the sort and the scroll place
                 of such a result survive a switch to another result and
                 back. An older result builds its grid again. -->
            <template v-for="pane in state.panes" :key="pane.id">
              <!-- A kept result belongs to an earlier run, so the cover of a
                   running statement lies over it alone. A result of the run
                   itself shows its rows while they arrive. -->
              <ResultsGrid
                v-if="livePaneIds.has(pane.id)"
                v-show="state.activePaneId === pane.id"
                :result="pane.result"
                :rows="pane.rows"
                :busy="state.running && pane.pinned"
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
                <v-icon v-if="message.level !== 'info'" size="x-small" class="mr-1">
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

    <!-- The bar stands in the place of the results panel while it is away.
         It names the result that is open and brings the panel back. -->
    <div
      v-if="resultsCollapsed"
      class="results-bar d-flex align-center px-2"
      data-test="results-bar"
    >
      <span class="text-caption text-medium-emphasis">{{ collapsedLabel }}</span>
      <v-spacer />
      <v-btn
        icon="mdi-chevron-up"
        size="small"
        variant="text"
        aria-label="Bring the results back"
        data-test="expand-results"
        @click="layout.setResultsCollapsed(false)"
      />
    </div>

    <AppDialog v-model="askingTable" max-width="420">
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
    </AppDialog>

    <AppDialog v-model="askingParams" max-width="520">
      <v-card ref="paramCard">
        <v-card-title class="text-subtitle-1">Give the values of the parameters</v-card-title>
        <v-card-text class="d-flex flex-column ga-3">
          <div class="text-caption text-medium-emphasis" data-test="parameters-help">
            Write <code>:name</code> in the statement to make a parameter.
          </div>
          <div
            v-for="row of paramRows"
            :key="row.name"
            class="d-flex align-center ga-2"
            data-test="parameter-row"
          >
            <div class="param-name text-medium-emphasis">:{{ row.name }}</div>
            <v-select
              :model-value="row.kind"
              :items="PARAM_KINDS"
              item-title="title"
              item-value="value"
              label="Form"
              hide-details
              class="param-kind"
              :data-test="`parameter-kind-${row.name}`"
              @update:model-value="(kind) => onParamKindChange(row, kind as ParamKind)"
            />
            <!-- A value of the true or false form takes one of two words, so
                 it is chosen and not written. -->
            <v-select
              v-if="row.kind === ParamKind.Boolean"
              v-model="row.text"
              :items="BOOLEAN_VALUES"
              label="Value"
              hide-details
              :data-test="`parameter-value-${row.name}`"
            />
            <v-text-field
              v-else
              v-model="row.text"
              :disabled="row.kind === ParamKind.Null"
              :error-messages="paramProblem(row) ?? undefined"
              label="Value"
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
            :disabled="paramsAreWrong"
            data-test="parameters-confirm"
            @click="confirmParams"
          />
        </v-card-actions>
      </v-card>
    </AppDialog>

    <ConfirmDialog
      :open="askingPlan"
      title="Run the statement for its plan"
      message="The actual plan comes from a real run. The statement runs on the server, so a
               statement that writes rows writes them, and a statement on Athena scans data."
      confirm-text="Run it"
      @confirm="confirmActualPlan"
      @cancel="askingPlan = false"
    />

    <AppDialog v-model="savingQuery" max-width="480">
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
    </AppDialog>
  </div>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import SqlEditor from './SqlEditor.vue'
import ResultsGrid from './ResultsGrid.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import { api } from '@/lib/api'
import { forgetTabActions, registerTabActions } from '@/lib/commands'
import { exportFileName, toCsv, toInsertStatements, toJson, toMarkdown } from '@/lib/export'
import { bytesToBase64, toXlsx } from '@/lib/xlsx'
import { formatClockTime, formatRowCount } from '@/lib/format'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { baseName, useFilesStore } from '@/stores/files'
import { useHistoryStore } from '@/stores/history'
import { MIN_EDITOR_SIZE, useLayoutStore } from '@/stores/layout'
import { newQueryState, useQueryStore } from '@/stores/query'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import { alignParams, needsAValue, paramChipLabel, paramProblem, paramsForRun } from '@/lib/params'
import { Dialect, ParamKind, PlanKind, type ParamValue, type ResultSet } from '@/types/api'
import type { ExportAllFormat, ExportFormat } from './ResultsGrid.vue'
import type { ResultPane } from '@/stores/query'
import type { QueryTab } from '@/stores/tabs'

/** The value that stands for the Messages tab. */
const MESSAGES_TAB = 'messages'

const props = defineProps<{ tab: QueryTab }>()

const tabs = useTabsStore()
const queries = useQueryStore()
const connections = useConnectionsStore()
const explorer = useExplorerStore()
const files = useFilesStore()
const history = useHistoryStore()
const layout = useLayoutStore()
const settings = useSettingsStore()
const ui = useUiStore()

const editorRef = ref<InstanceType<typeof SqlEditor> | null>(null)
/** The card of the parameter dialog, which a second request focuses. */
const paramCard = ref<{ $el: HTMLElement } | null>(null)
/**
 * The share the editor takes of the split. It lives in the layout store, so
 * every tab shows the same split and a restart brings it back.
 */
const editorSize = computed(() => layout.layout.editorSize)
/** True while the results panel is a bar below the editor. */
const resultsCollapsed = computed(() => layout.layout.resultsCollapsed)
/** True while the results panel stands below the editor and not beside it. */
const resultsBelow = computed(() => layout.layout.resultsOrientation === 'below')
const savingQuery = ref(false)
/** True while a write to the disk is on its way. */
const savingFile = ref(false)
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

// The state record is made here, outside the computed below, because a
// computed must read alone and this call writes a record for a new tab.
watch(
  () => props.tab.id,
  (id) => {
    queries.stateFor(id)
  },
  { immediate: true },
)
const state = computed(() => queries.states[props.tab.id] ?? newQueryState())

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
 * The result that is open, or nothing while the messages stand in its place.
 * The actions beside the tab strip act on this result.
 */
const activePane = computed(
  () => state.value.panes.find((pane) => pane.id === state.value.activePaneId) ?? null,
)

/**
 * The number of results that keep a grid. A grid holds the text of each cell
 * it draws, so many grids of a wide result take much memory. The rows stay in
 * the query store, and a result that loses its grid keeps them.
 */
const LIVE_PANE_LIMIT = 3

/** The results the user opened, with the result opened last at the front. */
const recentPaneIds = ref<string[]>([])

/** The results that keep a grid. A result that is closed drops out. */
const livePaneIds = computed(() => {
  const open = new Set(state.value.panes.map((pane) => pane.id))
  const live = new Set<string>()
  for (const id of recentPaneIds.value) {
    if (open.has(id)) {
      live.add(id)
    }
    if (live.size === LIVE_PANE_LIMIT) {
      break
    }
  }
  return live
})

watch(
  () => state.value.activePaneId,
  (id) => {
    if (id === null) {
      return
    }
    const open = new Set(state.value.panes.map((pane) => pane.id))
    recentPaneIds.value = [id, ...recentPaneIds.value.filter((old) => old !== id && open.has(old))]
  },
  { immediate: true },
)

/**
 * Names a result. A result the user keeps also carries the time of its run,
 * so that two results of the same statement can be told apart.
 */
/**
 * Brings the focus to the dialog that asks for the values of the parameters.
 * A second request for a run arrives while that dialog stands open, and the
 * dialog is what the user has to answer first.
 */
function focusParamDialog(): void {
  const card = paramCard.value?.$el as HTMLElement | undefined
  // The value of the first parameter is what the dialog waits for, so the
  // focus goes there and not to the kind that stands beside it.
  const field =
    card?.querySelector<HTMLElement>('[data-test^="parameter-value-"] input') ??
    card?.querySelector<HTMLElement>('input, button')
  field?.focus()
}

/** What the bar of the results panel says while the panel is away. */
const collapsedLabel = computed(() => (activePane.value ? paneLabel(activePane.value) : 'Messages'))

function paneLabel(pane: ResultPane): string {
  const name = pane.label ?? `Result ${pane.number}`
  const head = `${name} (${formatRowCount(pane.rows)})`
  return pane.pinned ? `${head} at ${formatClockTime(pane.ranAt)}` : head
}

/** The forms a value can take in the parameter dialog. */
const PARAM_KINDS = [
  { title: 'Text', value: ParamKind.Text },
  { title: 'Number', value: ParamKind.Number },
  { title: 'True or false', value: ParamKind.Boolean },
  { title: 'Empty value', value: ParamKind.Null },
]

/** The two words that a value of the true or false form takes. */
const BOOLEAN_VALUES = ['true', 'false']

/**
 * The wait before the names of the parameters are read again. The names come
 * from the backend, and a read for each letter that the user writes would
 * cross the bridge far too often.
 */
const PARAMETER_DEBOUNCE_MS = 300

/** The names that the statement of the tab holds, for the bar. */
const paramNames = ref<string[]>([])
let namesTimer: ReturnType<typeof setTimeout> | null = null

/** True while the value of one name is still missing. */
function paramIsUnset(name: string): boolean {
  const held = props.tab.params.find((value) => value.name === name)
  return !held || needsAValue(held)
}

/** Reads the names that the statement holds, for the bar above the editor. */
async function readParamNames(): Promise<void> {
  try {
    paramNames.value = await api.queryParameters(props.tab.query, dialect.value)
  } catch {
    // The bar is a help and not the run itself, so a failure to read the
    // names stays quiet. The run reports a failure of its own.
    paramNames.value = []
  }
}

// The names are read again after the user stops writing, and at once when
// the tab or its engine changes.
watch(
  () => [props.tab.id, props.tab.query, dialect.value] as const,
  (next, previous) => {
    if (namesTimer !== null) {
      clearTimeout(namesTimer)
    }
    // The first read and a move to another tab need the names at once. A
    // change of the text can wait until the user stops writing.
    if (next[0] !== previous?.[0]) {
      void readParamNames()
      return
    }
    namesTimer = setTimeout(() => {
      namesTimer = null
      void readParamNames()
    }, PARAMETER_DEBOUNCE_MS)
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  if (namesTimer !== null) {
    clearTimeout(namesTimer)
  }
})

/** True while one row of the dialog holds a text that its form refuses. */
const paramsAreWrong = computed(() => paramRows.value.some((row) => paramProblem(row) !== null))

/**
 * Answers a change of the form of one value. A value that becomes a true or
 * false value takes one of the two words, because the box that shows it holds
 * those two alone.
 */
function onParamKindChange(row: ParamValue, kind: ParamKind): void {
  row.kind = kind
  if (kind === ParamKind.Boolean && !BOOLEAN_VALUES.includes(row.text.trim().toLowerCase())) {
    row.text = 'false'
  }
}

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
    // One run at a time waits for its values. A second request would take
    // the place of the first one without a word.
    if (askingParams.value) {
      // The dialog the user already has is the answer to this request, so the
      // focus goes to it. A notice would say the same thing and leave the user
      // to find the dialog on their own.
      focusParamDialog()
      return
    }
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
  // A mark that started before the drag grows with the pointer, and the
  // library takes it away at the end of the drag alone. It goes on each step
  // instead, so no text of the editor stands out under the pointer.
  window.getSelection()?.removeAllRanges()
  if (panes[0]) {
    layout.setEditorSize(panes[0].size)
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
  // The rows of a run are what the user asked for, so the panel comes back.
  layout.setResultsCollapsed(false)
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
  void queries.cancel(props.tab.id)
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
async function onExportAll(format: ExportAllFormat): Promise<void> {
  const connectionId = props.tab.connectionId
  if (!connectionId) {
    return
  }
  // The export runs the statement of the last run, and not the text of the
  // editor, which the user may have changed since that run.
  const lastRun = state.value.lastRun
  if (!lastRun) {
    ui.warn('Run the statement first. The export writes the rows of a run.')
    return
  }
  try {
    const summary = await api.exportQuery({
      connectionId,
      requestId: `export-${props.tab.id}-${Date.now()}`,
      query: lastRun.query,
      defaultName: exportFileName(props.tab.title, format),
      format,
      maxRows: settings.settings.exportRowLimit,
      tabId: props.tab.id,
      queryParams: lastRun.params,
    })
    if (!summary) {
      return
    }
    if (summary.truncated) {
      ui.warn(
        `The export limit stopped the read at ${summary.rows.toLocaleString()} rows.`,
        'Raise the export limit in the settings.',
      )
    } else {
      ui.success(`${summary.rows.toLocaleString()} rows are written to ${summary.path}.`)
    }
  } catch (error) {
    ui.reportError(error)
  }
}

async function exportResult(result: ResultSet, format: ExportFormat): Promise<void> {
  const file = EXPORT_FILES[format]
  try {
    const request = {
      defaultName: exportFileName(props.tab.title, file.extension),
      filterLabel: file.label,
      extension: file.extension,
    }
    const path =
      format === 'xlsx'
        ? await api.saveBinaryFile({
            ...request,
            contents: bytesToBase64(toXlsx(result, props.tab.title)),
          })
        : await api.saveTextFile({ ...request, contents: textFor(result, format) })
    if (!path) {
      return
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

/** The name the save dialog suggests for a tab that holds no file yet. */
function suggestedFileName(title: string): string {
  return title.toLowerCase().endsWith('.sql') ? title : `${title}.sql`
}

/**
 * Writes the statement of the tab to a file. A tab that came from the disk
 * goes back to the same file. A tab without a file reaches the save dialog
 * of the operating system, which opens in the first folder of the files
 * panel when the panel holds one.
 */
async function saveToFile(): Promise<void> {
  if (savingFile.value) {
    return
  }
  savingFile.value = true
  try {
    const path = props.tab.filePath
    if (path) {
      await api.writeTextFile(path, props.tab.query)
      tabs.markClean(props.tab.id)
      ui.success(`The file ${baseName(path)} is written.`)
      return
    }
    const written = await api.saveStatementFile({
      defaultName: suggestedFileName(props.tab.title),
      defaultFolder: files.roots[0]?.path ?? null,
      contents: props.tab.query,
    })
    if (written === null) {
      return
    }
    tabs.setFilePath(props.tab.id, written)
    tabs.rename(props.tab.id, baseName(written))
    tabs.markClean(props.tab.id)
    ui.success(`The file ${baseName(written)} is written.`)
  } catch (error) {
    ui.reportError(error)
  } finally {
    savingFile.value = false
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

// The shell holds the keys, and the editor of this tab holds the text, so
// the view of each tab records what it can do under its own identifier.
onMounted(() => {
  registerTabActions(props.tab.id, {
    runStatement: () => runStatement(),
    runAll,
    cancel,
    format: formatStatement,
    save: () => {
      void saveToFile()
    },
  })
})

onBeforeUnmount(() => {
  forgetTabActions(props.tab.id)
})

defineExpose({ runStatement, runAll, formatStatement, readPlan, saveToFile })
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
  border-bottom: var(--app-divider);
  background: rgb(var(--v-theme-surface));
}

.param-name {
  min-width: 90px;
  font-family: var(--app-font-mono);
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

.parameter-bar {
  flex: 0 0 auto;
  border-bottom: var(--app-divider-soft);
  font-family: var(--app-font-mono);
}

.results-pane {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: rgb(var(--v-theme-surface));
}

.results-tab-row {
  flex: 0 0 auto;
  border-bottom: var(--app-divider);
}

.results-tabs {
  min-width: 0;
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

/* The detail of one message stays in the colour of its level, so a smaller
   size is what separates it from the text of the message. */
.message-detail {
  margin-left: 0.5rem;
  font-size: var(--app-text-sm);
}

.messages {
  overflow: auto;
  font-size: var(--app-text-md);
}

.message-line {
  padding: 2px 0;
  font-family: var(--app-font-mono);
}

.error-detail {
  margin-top: 6px;
  white-space: pre-wrap;
  font-size: var(--app-text-sm);
}

/* The splitter marks no text of its own. The library sets the state of a
   drag on the first move of the pointer, so a press on the splitter starts a
   mark of the browser before the rule for a drag can stop one. That mark
   then grows into the editor as the pointer moves. */
:deep(.splitpanes__splitter) {
  background: rgb(var(--v-theme-surface-variant));
  min-height: 4px;
  min-width: 4px;
  user-select: none;
}

/* The splitter has nothing to move while the results panel is away. */
.results-away :deep(.splitpanes__splitter) {
  display: none;
}

.results-bar {
  flex: 0 0 auto;
  height: 32px;
  border-top: var(--app-divider);
  background: rgb(var(--v-theme-surface));
}
</style>
