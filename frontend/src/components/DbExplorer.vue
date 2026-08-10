<template>
  <div class="db-explorer">
    <PanelHeader
      v-model:filter="explorer.filter"
      filter-placeholder="Filter objects"
      filter-label="Filter the objects"
      filter-test-id="explorer-filter"
    >
      <template #actions>
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
      </template>
    </PanelHeader>

    <v-progress-linear v-if="explorer.loading" indeterminate height="2" />

    <div class="explorer-body">
      <ExplorerTree
        v-if="explorer.visibleNodes.length > 0"
        :nodes="explorer.visibleNodes"
        :open-keys="openKeys"
        :selected-key="selectedKey"
        @activate="onActivate"
        @expand="onExpand"
        @collapse="onCollapse"
        @context="onContext"
      />

      <EmptyState v-else icon="mdi-database-off-outline" :title="emptyTitle" :hint="emptyHint">
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
      </EmptyState>
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
        <template v-if="isRelation(menuNode)">
          <v-list-item
            v-for="form of scriptForms"
            :key="form.kind"
            prepend-icon="mdi-script-text-outline"
            :title="form.title"
            :data-test="`menu-script-${form.kind}`"
            @click="scriptHere(menuNode, form.kind)"
          />
        </template>
        <v-list-item
          v-if="isRelation(menuNode)"
          prepend-icon="mdi-information-outline"
          title="Properties"
          data-test="menu-properties"
          @click="openProperties(menuNode)"
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

    <ConfirmDialog
      v-if="pendingDisconnect"
      :open="pendingDisconnect !== null"
      title="Close this connection?"
      :message="stoppedStatementsMessage(queries.runningOn(pendingDisconnect))"
      confirm-text="Close it"
      danger
      @confirm="confirmDisconnect(pendingDisconnect)"
      @cancel="pendingDisconnect = null"
    />

    <TableProperties
      :open="propertiesOpen"
      :node="propertiesNode"
      @close="propertiesOpen = false"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import ConfirmDialog from './ConfirmDialog.vue'
import EmptyState from './EmptyState.vue'
import ExplorerTree from './ExplorerTree.vue'
import PanelHeader from './PanelHeader.vue'
import TableProperties from './TableProperties.vue'
import { api } from '@/lib/api'
import { stoppedStatementsMessage } from '@/lib/format'
import { isExpandable, type ExplorerNode } from '@/stores/explorer'
import type { ScriptKind } from '@/types/api'
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
const propertiesOpen = ref(false)
const propertiesNode = ref<ExplorerNode | null>(null)
/** The connection that waits on an answer, while statements run on it. */
const pendingDisconnect = ref<string | null>(null)

/** Opens the properties of one relation. */
function openProperties(node: ExplorerNode): void {
  propertiesNode.value = node
  propertiesOpen.value = true
}

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
  if (openKeys.value.has(node.key)) {
    onCollapse(node)
    return
  }
  await onExpand(node)
}

/** Opens one branch and reads it. The tree asks only for a branch that is shut. */
async function onExpand(node: ExplorerNode): Promise<void> {
  const next = new Set(openKeys.value)
  next.add(node.key)
  openKeys.value = next
  await explorer.expand(node)
}

function onCollapse(node: ExplorerNode): void {
  const next = new Set(openKeys.value)
  next.delete(node.key)
  openKeys.value = next
}

function onContext({ x, y, node }: { x: number; y: number; node: ExplorerNode }): void {
  menu.open = false
  menu.x = x
  menu.y = y
  menu.node = node
  menu.open = true
}

async function refreshRoots(): Promise<void> {
  for (const root of explorer.roots) {
    await explorer.refresh(root)
  }
}

/**
 * Closes the connection of one node. A close stops every statement that runs
 * on the connection, so it asks first when one does.
 */
async function disconnectHere(node: ExplorerNode): Promise<void> {
  if (queries.runningOn(node.connectionId) > 0) {
    pendingDisconnect.value = node.connectionId
    return
  }
  await closeConnection(node.connectionId)
}

async function closeConnection(id: string): Promise<void> {
  await connections.disconnect(id)
  explorer.removeRoot(id)
}

async function confirmDisconnect(id: string): Promise<void> {
  pendingDisconnect.value = null
  await closeConnection(id)
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

/**
 * The four statements the menu offers for a table or a view. The CREATE form
 * says "draft" for an engine that keeps no text of its own, and the backend
 * decides which of the two the user gets.
 */
const scriptForms: { kind: ScriptKind; title: string }[] = [
  { kind: 'create', title: 'Script as CREATE' },
  { kind: 'select', title: 'Script as SELECT' },
  { kind: 'insert', title: 'Script as INSERT' },
  { kind: 'update', title: 'Script as UPDATE' },
]

/**
 * Puts the statement of one object in a new tab. The tab is never run,
 * because an INSERT or an UPDATE would change data.
 */
async function scriptHere(node: ExplorerNode, scriptKind: ScriptKind): Promise<void> {
  try {
    const statement = await api.scriptObject({
      connectionId: node.connectionId,
      database: node.database ?? null,
      schemaName: node.schema ?? null,
      tableName: node.table ?? node.label,
      kind: node.kind === 'view' ? 'view' : 'table',
      scriptKind,
    })
    tabs.add({
      connectionId: node.connectionId,
      query: statement,
      title: `${node.label} (${scriptKind})`,
    })
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

.explorer-body {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
  padding-top: 4px;
}
</style>
