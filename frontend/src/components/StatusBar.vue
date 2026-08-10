<template>
  <footer class="status-bar d-flex align-center ga-3 px-3">
    <div class="d-flex align-center ga-1" data-test="status-connection">
      <v-icon size="x-small" :color="healthColor">{{ healthIcon }}</v-icon>
      <span>{{ connectionLabel }}</span>
    </div>

    <v-divider vertical />

    <div data-test="status-state">{{ stateLabel }}</div>

    <template v-if="state && !state.running && state.results.length > 0">
      <v-divider vertical />
      <div data-test="status-rows">{{ formatRowCount(totalRows(state.results)) }}</div>
      <v-divider vertical />
      <div data-test="status-elapsed">{{ formatDuration(state.elapsedMs) }}</div>
    </template>

    <template v-if="state && state.rowsAffected !== null">
      <v-divider vertical />
      <div data-test="status-affected">{{ formatRowCount(state.rowsAffected) }} affected</div>
    </template>

    <v-spacer />

    <div v-if="tab" data-test="status-dialect">{{ dialectLabel }}</div>
  </footer>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { formatDuration, formatRowCount } from '@/lib/format'
import { useConnectionsStore } from '@/stores/connections'
import { useQueryStore } from '@/stores/query'
import { useTabsStore } from '@/stores/tabs'
import { totalRows } from '@/stores/query'
import { ConnectionHealth, Dialect } from '@/types/api'

const connections = useConnectionsStore()
const tabs = useTabsStore()
const queries = useQueryStore()

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
  return connections.byId(id)?.name ?? id
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
  font-size: 0.75rem;
  background: rgb(var(--v-theme-surface-light));
  border-top: 1px solid rgb(var(--v-theme-surface-variant));
  color: rgb(var(--v-theme-on-surface-variant));
}
</style>
