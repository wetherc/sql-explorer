<template>
  <div class="history-panel">
    <PanelHeader
      v-model:filter="history.filter"
      filter-label="Filter statements"
      filter-test-id="history-filter"
    >
      <template #switch>
        <v-btn-toggle v-model="mode" density="compact" mandatory divided>
          <v-btn value="history" size="small" text="History" data-test="mode-history" />
          <v-btn value="saved" size="small" text="Saved queries" data-test="mode-saved" />
        </v-btn-toggle>
      </template>
      <template #actions>
        <v-tooltip v-if="mode === 'history'" location="bottom" text="Clear history">
          <template #activator="{ props: tip }">
            <v-btn
              v-bind="tip"
              icon="mdi-delete-sweep-outline"
              size="small"
              aria-label="Clear history"
              :disabled="history.entries.length === 0"
              data-test="clear-history"
              @click="clearing = true"
            />
          </template>
        </v-tooltip>
      </template>
    </PanelHeader>

    <div ref="bodyRef" class="body">
      <template v-if="mode === 'history'">
        <!-- The list draws the rows that stand in the window alone, because
             the history holds up to five hundred entries. -->
        <v-list v-if="history.visibleEntries.length > 0" density="compact" class="pa-0">
          <v-virtual-scroll
            :items="history.visibleEntries"
            :item-height="ITEM_HEIGHT"
            :height="listHeight"
          >
            <template #default="{ item: entry }">
              <v-list-item :key="entry.id" data-test="history-entry" @click="openEntry(entry)">
                <template #prepend>
                  <v-icon
                    size="small"
                    :color="entry.succeeded ? 'success' : 'error'"
                    aria-hidden="true"
                  >
                    {{ entry.succeeded ? 'mdi-check' : 'mdi-alert-circle-outline' }}
                  </v-icon>
                </template>
                <v-list-item-title class="query-line" :title="entryTip(entry)">
                  <span class="d-sr-only">{{ entry.succeeded ? 'Succeeded: ' : 'Failed: ' }}</span>
                  {{ summariseQuery(entry.query) }}
                </v-list-item-title>
                <v-list-item-subtitle v-if="entry.succeeded">
                  {{ entry.connectionName }} · {{ formatTimestamp(entry.ranAt) }} ·
                  {{ formatDuration(entry.elapsedMs) }} · {{ formatRowCount(entry.rowCount) }}
                </v-list-item-subtitle>
                <v-list-item-subtitle v-else class="failed-line" data-test="history-error">
                  {{ entry.connectionName }} · {{ formatTimestamp(entry.ranAt) }} ·
                  {{ entry.error || 'Failed' }}
                </v-list-item-subtitle>
              </v-list-item>
            </template>
          </v-virtual-scroll>
        </v-list>
        <EmptyState
          v-else-if="history.entries.length > 0"
          icon="mdi-magnify"
          title="No matches"
          hint="No statement in the history matches the filter."
        >
          <v-btn
            size="small"
            variant="tonal"
            text="Clear filter"
            data-test="history-clear-filter"
            @click="history.filter = ''"
          />
        </EmptyState>
        <EmptyState
          v-else
          icon="mdi-history"
          title="No history yet"
          hint="Statements you run show up here, along with how long they took."
        />
      </template>

      <template v-else>
        <v-list v-if="history.visibleSavedQueries.length > 0" density="compact" class="pa-0">
          <v-virtual-scroll
            :items="history.visibleSavedQueries"
            :item-height="ITEM_HEIGHT"
            :height="listHeight"
          >
            <template #default="{ item: query }">
              <v-list-item :key="query.id" data-test="saved-entry" @click="openSaved(query)">
                <template #prepend>
                  <v-icon size="small">mdi-bookmark-outline</v-icon>
                </template>
                <v-list-item-title>{{ query.name }}</v-list-item-title>
                <v-list-item-subtitle class="query-line">
                  {{ summariseQuery(query.query) }}
                </v-list-item-subtitle>
                <template #append>
                  <v-tooltip location="bottom" text="Delete saved query">
                    <template #activator="{ props: tip }">
                      <v-btn
                        v-bind="tip"
                        icon="mdi-delete-outline"
                        size="x-small"
                        class="row-action"
                        aria-label="Delete saved query"
                        data-test="delete-saved"
                        @click.stop="pendingDelete = query"
                      />
                    </template>
                  </v-tooltip>
                </template>
              </v-list-item>
            </template>
          </v-virtual-scroll>
        </v-list>
        <EmptyState
          v-else-if="history.savedQueries.length > 0"
          icon="mdi-magnify"
          title="No matches"
          hint="No saved query matches the filter."
        >
          <v-btn
            size="small"
            variant="tonal"
            text="Clear filter"
            data-test="saved-clear-filter"
            @click="history.filter = ''"
          />
        </EmptyState>
        <EmptyState
          v-else
          icon="mdi-bookmark-outline"
          title="No saved queries"
          hint="Save a query from its tab to reopen it later."
        />
      </template>
    </div>

    <ConfirmDialog
      :open="clearing"
      title="Clear history?"
      message="This removes every statement from the history. Saved queries are kept."
      confirm-text="Clear"
      danger
      @confirm="confirmClear"
      @cancel="clearing = false"
    />

    <ConfirmDialog
      :open="pendingDelete !== null"
      title="Delete this saved query?"
      :message="`This deletes the saved query ${pendingDelete?.name ?? ''}.`"
      confirm-text="Delete"
      danger
      @confirm="confirmDelete"
      @cancel="pendingDelete = null"
    />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
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

/** The height of one row of either list. */
const ITEM_HEIGHT = 56

/** The element that holds the list, which gives the height of the window. */
const bodyRef = ref<HTMLElement | null>(null)
/**
 * The height of the window of the list. The value stands here and not in the
 * style, because the list needs a number to count the rows it draws. The
 * first value serves a place where nothing is laid out.
 */
const listHeight = ref(600)
let sizeObserver: ResizeObserver | null = null

onMounted(() => {
  const element = bodyRef.value
  if (!element || typeof ResizeObserver === 'undefined') {
    return
  }
  sizeObserver = new ResizeObserver(() => {
    listHeight.value = element.clientHeight || listHeight.value
  })
  sizeObserver.observe(element)
})

onBeforeUnmount(() => {
  sizeObserver?.disconnect()
  sizeObserver = null
})

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

/** The full text of one entry, with the reason of a failure. */
function entryTip(entry: HistoryEntry): string {
  return entry.succeeded ? entry.query : `${entry.query}\n\n${entry.error || 'Failed'}`
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
  /* The list of the rows scrolls itself, so this element does not. */
  overflow: hidden;
  min-height: 0;
}

.failed-line {
  color: rgb(var(--v-theme-error));
}

/* The delete button of a row shows while the pointer or the focus is on the
   row, so a list of saved queries is not a column of red buttons. */
.row-action {
  opacity: 0;
}

:deep(.v-list-item:hover) .row-action,
:deep(.v-list-item:focus-within) .row-action,
.row-action:focus-visible {
  opacity: 1;
}

.query-line {
  font-family: var(--app-font-mono);
  font-size: var(--app-text-sm);
}
</style>
