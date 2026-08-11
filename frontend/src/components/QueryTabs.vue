<template>
  <div class="query-tabs">
    <div class="tab-strip d-flex align-center">
      <v-tabs
        :model-value="tabs.activeTabId"
        density="compact"
        show-arrows
        class="flex-grow-1"
        @update:model-value="(id) => tabs.activate(String(id))"
      >
        <!-- A tab is itself a button, so the close mark inside it cannot be a
             second button. The mark answers the mouse, and the Delete key on
             the tab closes it for a user of the keyboard. -->
        <v-tab
          v-for="tab in tabs.tabs"
          :key="tab.id"
          :value="tab.id"
          class="query-tab"
          data-test="query-tab"
          @keydown.delete.prevent="askClose(tab)"
          @dblclick.stop.prevent="startRename(tab)"
        >
          <!-- The field stops its own pointer events, so an edit does not
               fight the activation of the tab under it. -->
          <input
            v-if="renamingId === tab.id"
            ref="titleField"
            v-model="renameText"
            class="tab-title-field"
            aria-label="The name of the tab"
            data-test="tab-title-field"
            @click.stop
            @mousedown.stop
            @dblclick.stop
            @keydown.stop
            @keydown.enter.prevent="commitRename"
            @keydown.esc.prevent="cancelRename"
            @blur="commitRename"
          />
          <span v-else class="tab-title">{{ tab.title }}</span>
          <span v-if="tab.dirty" class="dirty-mark" aria-hidden="true">●</span>
          <span v-if="tab.dirty" class="app-visually-hidden">, has changes</span>
          <v-icon
            size="x-small"
            class="ml-2 close-mark"
            aria-hidden="true"
            data-test="close-tab"
            @click.stop="askClose(tab)"
          >
            mdi-close
          </v-icon>
        </v-tab>
      </v-tabs>
      <v-tooltip location="bottom" text="Open a new tab">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            icon="mdi-plus"
            size="small"
            aria-label="Open a new tab"
            data-test="new-tab"
            @click="newTab"
          />
        </template>
      </v-tooltip>
    </div>

    <div class="tab-body">
      <!-- A tab mounts its view, with its editor, the first time the user
           opens it. Only the tabs the user opened last keep their view. A
           workspace with many tabs then holds few editors. -->
      <template v-for="tab in tabs.tabs" :key="tab.id">
        <QueryView v-if="liveTabIds.has(tab.id)" v-show="tab.id === tabs.activeTabId" :tab="tab" />
      </template>

      <EmptyState
        v-if="!tabs.hasTabs"
        size="page"
        icon="mdi-database-search-outline"
        title="No open tabs"
        :hint="emptyHint"
      >
        <v-btn
          v-if="connections.hasActive"
          color="primary"
          variant="flat"
          prepend-icon="mdi-plus"
          text="New query"
          data-test="empty-new-tab"
          @click="newTab"
        />
        <v-btn
          v-else
          color="primary"
          variant="flat"
          prepend-icon="mdi-lan-connect"
          text="Open the connections"
          data-test="empty-open-connections"
          @click="emit('open-connections')"
        />
      </EmptyState>
    </div>

    <ConfirmDialog
      :open="pendingClose !== null"
      title="Close this tab?"
      :message="`The changes to ${pendingClose?.title ?? ''} are not saved, and closing the tab loses them.`"
      confirm-text="Close the tab"
      danger
      @confirm="confirmClose"
      @cancel="pendingClose = null"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import ConfirmDialog from './ConfirmDialog.vue'
import EmptyState from './EmptyState.vue'
import QueryView from './QueryView.vue'
import { useConnectionsStore } from '@/stores/connections'
import { useTabsStore, type QueryTab } from '@/stores/tabs'

const tabs = useTabsStore()
const connections = useConnectionsStore()

/**
 * The number of tabs that keep their view. A view holds an editor and the
 * grids of its results, which together take much memory. The text of a tab,
 * its results and its messages stay in the stores, so a tab that loses its
 * view keeps its work. The editor of such a tab loses the list of undo steps
 * and the place of the cursor.
 */
const LIVE_TAB_LIMIT = 5

/** The tabs the user opened, with the tab opened last at the front. */
const recentTabIds = ref<string[]>([])

/** The tabs that keep their view. A tab that is not open drops out. */
const liveTabIds = computed(() => {
  const open = new Set(tabs.tabs.map((tab) => tab.id))
  const live = new Set<string>()
  for (const id of recentTabIds.value) {
    if (open.has(id)) {
      live.add(id)
    }
    if (live.size === LIVE_TAB_LIMIT) {
      break
    }
  }
  return live
})

watch(
  () => tabs.activeTabId,
  (id) => {
    if (id === null) {
      return
    }
    const open = new Set(tabs.tabs.map((tab) => tab.id))
    recentTabIds.value = [id, ...recentTabIds.value.filter((old) => old !== id && open.has(old))]
  },
  { immediate: true },
)

const emit = defineEmits<{ (event: 'open-connections'): void }>()

const emptyHint = computed(() =>
  connections.hasActive
    ? 'Open a tab to write a statement against the connection you selected.'
    : 'Open a connection first. Its objects then appear in the explorer.',
)

function newTab(): void {
  tabs.add()
}

/** The tab whose name the user edits, and the text of that edit. */
const renamingId = ref<string | null>(null)
const renameText = ref('')
// The field sits inside a loop, so Vue gathers its element into an array.
const titleField = ref<HTMLInputElement[]>([])

/**
 * Starts an edit of the name of a tab. The tab that the edit belongs to
 * becomes the active tab, because the edit stands inside it.
 */
function startRename(tab: QueryTab): void {
  tabs.activate(tab.id)
  renamingId.value = tab.id
  renameText.value = tab.title
  void nextTick(() => {
    const field = titleField.value[0]
    field?.focus()
    field?.select()
  })
}

/**
 * Writes the name the edit holds. An empty text keeps the name that the tab
 * already carries, which the store enforces as well.
 */
function commitRename(): void {
  const id = renamingId.value
  if (id === null) {
    return
  }
  renamingId.value = null
  tabs.rename(id, renameText.value)
}

function cancelRename(): void {
  renamingId.value = null
}

/** Starts an edit of the name of the tab that stands open. */
function renameActiveTab(): void {
  const tab = tabs.tabs.find((item) => item.id === tabs.activeTabId)
  if (tab) {
    startRename(tab)
  }
}

defineExpose({ renameActiveTab })

/** The tab that waits on an answer, while its changes are not saved. */
const pendingClose = ref<QueryTab | null>(null)

/**
 * Closes one tab. A tab whose changes are not saved asks first, because the
 * text of the statement is lost with it.
 */
function askClose(tab: QueryTab): void {
  if (tab.dirty) {
    pendingClose.value = tab
    return
  }
  tabs.close(tab.id)
}

function confirmClose(): void {
  const tab = pendingClose.value
  pendingClose.value = null
  if (tab) {
    tabs.close(tab.id)
  }
}
</script>

<style scoped>
.query-tabs {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.tab-strip {
  flex: 0 0 auto;
  background: rgb(var(--v-theme-surface-light));
  border-bottom: var(--app-divider);
}

/* The body holds the height that remains, so the views inside it can size
   themselves against it. Without this rule the body grows with the rows of
   a result, and the split of the query view slides below the window. */
.tab-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.tab-body > * {
  flex: 1 1 auto;
  min-height: 0;
}

.query-tab {
  text-transform: none;
  letter-spacing: normal;
}

.tab-title {
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* The field takes the place of the label, so the tab keeps its width while
   the user writes in it. */
.tab-title-field {
  max-width: 200px;
  min-width: 80px;
  color: inherit;
  font: inherit;
  background: rgba(var(--v-theme-on-surface), 0.08);
  border-radius: 3px;
  padding: 0 4px;
  outline: none;
}

.dirty-mark {
  margin-left: 6px;
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-warning));
}

/* The mark grows a background under the pointer, so the area it answers is
   plain to see before the click. */
.close-mark {
  border-radius: 50%;
  padding: 2px;
}

.close-mark:hover {
  background: rgba(var(--v-theme-on-surface), 0.12);
}

.tab-body {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
}

.tab-body > * {
  height: 100%;
}
</style>
