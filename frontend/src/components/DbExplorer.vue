<template>
  <div class="db-explorer">
    <div class="explorer-header d-flex align-center ga-1 px-2 py-1">
      <v-text-field
        v-model="explorer.filter"
        density="compact"
        hide-details
        clearable
        placeholder="Filter objects"
        prepend-inner-icon="mdi-magnify"
        data-test="explorer-filter"
      />
      <v-tooltip location="bottom" text="Read the objects again">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            icon="mdi-refresh"
            size="small"
            aria-label="Read the objects again"
            data-test="explorer-refresh"
            @click="refreshRoots"
          />
        </template>
      </v-tooltip>
    </div>

    <v-progress-linear v-if="explorer.loading" indeterminate height="2" />

    <div class="explorer-body">
      <ExplorerTree
        v-if="explorer.visibleNodes.length > 0"
        :nodes="explorer.visibleNodes"
        :open-keys="openKeys"
        :selected-key="selectedKey"
        @activate="onActivate"
        @context="onContext"
      />

      <div v-else class="empty-state pa-6 text-center">
        <v-icon size="44" class="mb-3 text-medium-emphasis">mdi-database-off-outline</v-icon>
        <div class="text-body-2 mb-1">{{ emptyTitle }}</div>
        <p class="text-caption text-medium-emphasis mb-4">{{ emptyHint }}</p>
        <v-btn
          v-if="!connections.hasActive"
          color="primary"
          variant="flat"
          size="small"
          prepend-icon="mdi-lan-connect"
          text="Open a connection"
          data-test="explorer-open-connections"
          @click="emit('open-connections')"
        />
      </div>
    </div>

    <v-menu v-model="menu.open" :target="[menu.x, menu.y]" data-test="explorer-menu">
      <v-list v-if="menuNode" density="compact" min-width="220">
        <v-list-item
          v-if="isRelation(menuNode)"
          prepend-icon="mdi-table-eye"
          title="Select the first 1000 rows"
          data-test="menu-preview"
          @click="previewRows(menuNode)"
        />
        <v-list-item
          v-if="isRelation(menuNode)"
          prepend-icon="mdi-format-list-bulleted"
          title="Copy the name"
          data-test="menu-copy-name"
          @click="copyName(menuNode)"
        />
        <v-list-item
          prepend-icon="mdi-file-document-outline"
          title="New query on this connection"
          data-test="menu-new-query"
          @click="tabs.add({ connectionId: menuNode.connectionId })"
        />
        <v-divider />
        <v-list-item
          v-if="isExpandable(menuNode)"
          prepend-icon="mdi-refresh"
          title="Read this branch again"
          data-test="menu-refresh"
          @click="explorer.refresh(menuNode)"
        />
        <v-list-item
          v-if="menuNode.kind === 'connection'"
          prepend-icon="mdi-lan-disconnect"
          title="Close this connection"
          data-test="menu-disconnect"
          @click="disconnectHere(menuNode)"
        />
      </v-list>
    </v-menu>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import ExplorerTree from './ExplorerTree.vue'
import { api } from '@/lib/api'
import { isExpandable, type ExplorerNode } from '@/stores/explorer'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useQueryStore } from '@/stores/query'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'

const explorer = useExplorerStore()
const connections = useConnectionsStore()
const tabs = useTabsStore()
const queries = useQueryStore()
const settings = useSettingsStore()
const ui = useUiStore()

const emit = defineEmits<{ (event: 'open-connections'): void }>()

const openKeys = ref(new Set<string>())
const selectedKey = ref<string | null>(null)
const menu = reactive({ open: false, x: 0, y: 0, node: null as ExplorerNode | null })

/**
 * The node the menu belongs to. The menu draws nothing without one, so
 * every action below receives a node and needs no guard of its own.
 */
const menuNode = computed(() => menu.node)

const emptyTitle = computed(() =>
  connections.hasActive ? 'Nothing matches the filter' : 'No open connection',
)
const emptyHint = computed(() =>
  connections.hasActive
    ? 'Clear the filter to see the whole tree.'
    : 'Open a connection to see its databases, tables and columns.',
)

function isRelation(node: ExplorerNode): boolean {
  return node.kind === 'table' || node.kind === 'view'
}

async function onActivate(node: ExplorerNode): Promise<void> {
  selectedKey.value = node.key
  if (!isExpandable(node)) {
    return
  }
  const next = new Set(openKeys.value)
  if (next.has(node.key)) {
    next.delete(node.key)
    openKeys.value = next
    return
  }
  next.add(node.key)
  openKeys.value = next
  await explorer.expand(node)
}

function onContext({ event, node }: { event: MouseEvent; node: ExplorerNode }): void {
  menu.open = false
  menu.x = event.clientX
  menu.y = event.clientY
  menu.node = node
  menu.open = true
}

async function refreshRoots(): Promise<void> {
  for (const root of explorer.roots) {
    await explorer.refresh(root)
  }
}

async function disconnectHere(node: ExplorerNode): Promise<void> {
  await connections.disconnect(node.connectionId)
  explorer.removeRoot(node.connectionId)
}

/**
 * Asks the backend to build the statement, so that every name is quoted
 * for the engine and no dialect rule lives in the interface.
 */
async function previewRows(node: ExplorerNode): Promise<void> {
  try {
    const statement = await api.previewQuery({
      connectionId: node.connectionId,
      database: node.database ?? null,
      schemaName: node.schema ?? null,
      tableName: node.table ?? node.label,
      limit: 1000,
    })
    const tab = tabs.add({
      connectionId: node.connectionId,
      query: statement,
      title: node.label,
    })
    if (settings.settings.autoRunPreview) {
      await queries.execute(tab.id, node.connectionId, statement)
    }
  } catch (error) {
    ui.reportError(error)
  }
}

async function copyName(node: ExplorerNode): Promise<void> {
  try {
    const quoted = await api.quoteIdentifier(node.connectionId, node.table ?? node.label)
    const clipboard = globalThis.navigator?.clipboard
    if (clipboard) {
      await clipboard.writeText(quoted)
    }
    ui.success('The name is on the clipboard.')
  } catch (error) {
    ui.reportError(error)
  }
}
</script>

<style scoped>
.db-explorer {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.explorer-header {
  flex: 0 0 auto;
  border-bottom: 1px solid rgb(var(--v-theme-surface-variant));
}

.explorer-body {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
  padding-top: 4px;
}
</style>
