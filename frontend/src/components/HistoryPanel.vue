<template>
  <div class="history-panel">
    <PanelHeader
      v-model:filter="history.filter"
      filter-label="Filter the statements"
      filter-test-id="history-filter"
    >
      <template #switch>
        <v-btn-toggle v-model="mode" density="compact" mandatory divided>
          <v-btn value="history" size="small" text="History" data-test="mode-history" />
          <v-btn value="saved" size="small" text="Saved" data-test="mode-saved" />
        </v-btn-toggle>
      </template>
      <template #actions>
        <v-tooltip v-if="mode === 'history'" location="bottom" text="Empty the history">
          <template #activator="{ props: tip }">
            <v-btn
              v-bind="tip"
              icon="mdi-delete-sweep-outline"
              size="small"
              aria-label="Empty the history"
              data-test="clear-history"
              @click="clearing = true"
            />
          </template>
        </v-tooltip>
      </template>
    </PanelHeader>

    <div class="body">
      <template v-if="mode === 'history'">
        <v-list v-if="history.visibleEntries.length > 0" density="compact" class="pa-0">
          <v-list-item
            v-for="entry in history.visibleEntries"
            :key="entry.id"
            data-test="history-entry"
            @click="openEntry(entry)"
          >
            <template #prepend>
              <v-icon
                size="small"
                :color="entry.succeeded ? 'success' : 'error'"
                :aria-label="entry.succeeded ? 'succeeded' : 'failed'"
              >
                {{ entry.succeeded ? 'mdi-check' : 'mdi-alert-circle-outline' }}
              </v-icon>
            </template>
            <v-list-item-title class="query-line">
              {{ summariseQuery(entry.query) }}
            </v-list-item-title>
            <v-list-item-subtitle>
              {{ entry.connectionName }} · {{ formatTimestamp(entry.ranAt) }} ·
              {{ formatDuration(entry.elapsedMs) }} · {{ formatRowCount(entry.rowCount) }}
            </v-list-item-subtitle>
          </v-list-item>
        </v-list>
        <EmptyState
          v-else
          icon="mdi-history"
          title="No statement has run yet"
          hint="Every statement you run appears here, with the time it took."
        />
      </template>

      <template v-else>
        <v-list v-if="history.visibleSavedQueries.length > 0" density="compact" class="pa-0">
          <v-list-item
            v-for="query in history.visibleSavedQueries"
            :key="query.id"
            data-test="saved-entry"
            @click="openSaved(query)"
          >
            <template #prepend>
              <v-icon size="small">mdi-bookmark-outline</v-icon>
            </template>
            <v-list-item-title>{{ query.name }}</v-list-item-title>
            <v-list-item-subtitle class="query-line">
              {{ summariseQuery(query.query) }}
            </v-list-item-subtitle>
            <template #append>
              <v-btn
                icon="mdi-delete"
                size="x-small"
                color="error"
                aria-label="Delete the saved statement"
                data-test="delete-saved"
                @click.stop="pendingDelete = query"
              />
            </template>
          </v-list-item>
        </v-list>
        <EmptyState
          v-else
          icon="mdi-bookmark-outline"
          title="No statement is saved yet"
          hint="Save a statement from its tab to open it again later."
        />
      </template>
    </div>

    <ConfirmDialog
      :open="clearing"
      title="Empty the history?"
      message="Every statement that has run is taken away. Saved statements stay."
      confirm-text="Empty it"
      danger
      @confirm="confirmClear"
      @cancel="clearing = false"
    />

    <ConfirmDialog
      :open="pendingDelete !== null"
      title="Delete this saved statement?"
      :message="`The statement named ${pendingDelete?.name ?? ''} is taken away.`"
      confirm-text="Delete"
      danger
      @confirm="confirmDelete"
      @cancel="pendingDelete = null"
    />
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import ConfirmDialog from './ConfirmDialog.vue'
import EmptyState from './EmptyState.vue'
import PanelHeader from './PanelHeader.vue'
import { formatDuration, formatRowCount, formatTimestamp, summariseQuery } from '@/lib/format'
import { useHistoryStore } from '@/stores/history'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionsStore } from '@/stores/connections'
import type { HistoryEntry, SavedQuery } from '@/types/api'

const history = useHistoryStore()
const tabs = useTabsStore()
const connections = useConnectionsStore()

const mode = ref<'history' | 'saved'>('history')

/** True while the question about emptying the history stands open. */
const clearing = ref(false)
/** The saved statement that waits on an answer about its deletion. */
const pendingDelete = ref<SavedQuery | null>(null)

function confirmClear(): void {
  clearing.value = false
  history.clear()
}

function confirmDelete(): void {
  const query = pendingDelete.value
  pendingDelete.value = null
  if (query) {
    history.remove(query.id)
  }
}

/** Opens a past statement in a new tab, on the connection it ran against. */
function openEntry(entry: HistoryEntry): void {
  const connectionId = connections.isActive(entry.connectionId)
    ? entry.connectionId
    : connections.selectedId
  tabs.add({ connectionId, query: entry.query })
}

function openSaved(query: SavedQuery): void {
  const connectionId =
    query.connectionId && connections.isActive(query.connectionId)
      ? query.connectionId
      : connections.selectedId
  tabs.add({ connectionId, query: query.query, title: query.name })
}
</script>

<style scoped>
.history-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.body {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
}

.query-line {
  font-family: var(--app-font-mono);
  font-size: var(--app-text-sm);
}
</style>
