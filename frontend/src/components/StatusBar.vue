<template>
  <footer class="status-bar d-flex align-center ga-3 px-3">
    <div class="d-flex align-center ga-1" data-test="status-connection">
      <v-icon size="x-small" :color="healthColor">{{ healthIcon }}</v-icon>
      <span>{{ connectionLabel }}</span>
    </div>

    <v-divider vertical />

    <!-- The state of a run changes with no other sign, so a reader is told of
         each change as it comes. -->
    <div role="status" aria-live="polite" data-test="status-state">{{ stateLabel }}</div>

    <!-- While the statement runs, the time that has passed stands in the
         place the final time takes later, so a reader watches one figure. -->
    <template v-if="runningMs !== null">
      <v-divider vertical />
      <div data-test="status-running-elapsed">{{ formatDuration(runningMs) }}</div>
    </template>

    <template v-if="state && !state.running && state.panes.length > 0">
      <v-divider vertical />
      <div data-test="status-rows">{{ formatRowCount(totalRows(resultsOf(state.panes))) }}</div>
      <v-divider vertical />
      <div data-test="status-elapsed">{{ formatDuration(state.elapsedMs) }}</div>
    </template>

    <template v-if="state && state.rowsAffected !== null">
      <v-divider vertical />
      <div data-test="status-affected">{{ formatRowCount(state.rowsAffected) }} affected</div>
    </template>

    <template v-if="scannedBytes !== null">
      <v-divider vertical />
      <v-tooltip location="top" :text="scanTooltip">
        <template #activator="{ props: tip }">
          <div v-bind="tip" data-test="status-scan">
            {{ formatBytes(scannedBytes) }} scanned, {{ formatCost(lastCost) }} est.
          </div>
        </template>
      </v-tooltip>
      <v-divider vertical />
      <div data-test="status-session-cost">{{ formatCost(sessionCost) }} this session</div>
    </template>

    <v-spacer />

    <div v-if="tab" data-test="status-dialect">{{ dialectLabel }}</div>
  </footer>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { formatBytes, formatCost, formatDuration, formatRowCount, scanCost } from '@/lib/format'
import { useConnectionsStore } from '@/stores/connections'
import { useQueryStore } from '@/stores/query'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { resultsOf, totalRows } from '@/stores/query'
import { ConnectionHealth, Dialect } from '@/types/api'

const connections = useConnectionsStore()
const tabs = useTabsStore()
const queries = useQueryStore()
const settings = useSettingsStore()

const tab = computed(() => tabs.activeTab)
const state = computed(() => (tab.value ? queries.stateFor(tab.value.id) : null))

const connectionId = computed(() => tab.value?.connectionId ?? connections.selectedId)

const health = computed(() =>
  connectionId.value ? connections.health[connectionId.value] : undefined,
)

const healthColor = computed(() => {
  switch (health.value) {
    case ConnectionHealth.Connected:
      return 'success'
    case ConnectionHealth.Reconnecting:
      return 'warning'
    default:
      return 'error'
  }
})

const healthIcon = computed(() =>
  health.value === ConnectionHealth.Connected ? 'mdi-lan-connect' : 'mdi-lan-disconnect',
)

const connectionLabel = computed(() => {
  const id = connectionId.value
  if (!id) {
    return 'No connection'
  }
  return connections.nameFor(id)
})

const stateLabel = computed(() => {
  if (!state.value) {
    return 'Ready'
  }
  if (state.value.running) {
    return 'Running…'
  }
  if (state.value.error) {
    return `Failed: ${state.value.error.kind}`
  }
  return 'Ready'
})

/**
 * How often the time of a running statement is drawn again. A statement that
 * runs for minutes needs no faster reading than this, and the bar draws
 * itself alone.
 */
const RUNNING_TICK_MS = 100

/** The moment the clock of a running statement last read. */
const now = ref(Date.now())
let ticker: ReturnType<typeof setInterval> | null = null

/** The moment the statement of this tab started, or `null` when none runs. */
const runningSince = computed(() => (state.value?.running ? (state.value.startedAt ?? null) : null))

/** The time that has passed since the statement started. */
const runningMs = computed(() => {
  const since = runningSince.value
  return since === null ? null : Math.max(0, now.value - since)
})

function stopTicker(): void {
  if (ticker !== null) {
    clearInterval(ticker)
    ticker = null
  }
}

// The clock runs only while a statement runs, so an idle window holds no
// timer of its own.
watch(
  runningSince,
  (since) => {
    stopTicker()
    if (since === null) {
      return
    }
    now.value = Date.now()
    ticker = setInterval(() => {
      now.value = Date.now()
    }, RUNNING_TICK_MS)
  },
  { immediate: true },
)

onBeforeUnmount(stopTicker)

/** The bytes the last statement of this tab scanned, when it reported them. */
const scannedBytes = computed(() => state.value?.stats?.scannedBytes ?? null)

const lastCost = computed(() =>
  scanCost(scannedBytes.value ?? 0, settings.settings.athenaPricePerTerabyte),
)

const sessionCost = computed(() =>
  scanCost(queries.sessionScannedBytes, settings.settings.athenaPricePerTerabyte),
)

const scanTooltip = computed(() => {
  const rate = `The rate is $${settings.settings.athenaPricePerTerabyte} for each terabyte.`
  const reused = state.value?.stats?.resultReused
    ? ' The engine gave the result of an earlier run, which costs nothing.'
    : ''
  return `An estimate. ${rate}${reused}`
})

const dialectLabel = computed(() => {
  const id = connectionId.value
  const dialect = id ? connections.active[id]?.dialect : undefined
  switch (dialect) {
    case Dialect.MsSql:
      return 'T-SQL'
    case Dialect.MySql:
      return 'MySQL'
    case Dialect.Postgres:
      return 'PostgreSQL'
    case Dialect.Sqlite:
      return 'SQLite'
    case Dialect.Athena:
      return 'Athena'
    default:
      return 'SQL'
  }
})
</script>

<style scoped>
.status-bar {
  height: 26px;
  flex: 0 0 auto;
  font-size: var(--app-text-sm);
  background: rgb(var(--v-theme-surface-light));
  border-top: var(--app-divider);
  color: rgb(var(--v-theme-on-surface-variant));
}
</style>
