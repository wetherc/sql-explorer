<template>
  <div>
    <v-progress-linear v-if="explorerStore.loading" indeterminate></v-progress-linear>
    <AppTreeview :nodes="treeviewNodes" @expand="onExpand" @node-click="onNodeClick" @contextmenu="onContextMenu" />

    <v-menu
      v-model="contextMenu.visible"
      :style="{ top: `${contextMenu.y}px`, left: `${contextMenu.x}px` }"
      absolute
    >
      <v-list dense>
        <v-list-item @click="handleContextMenuAction('new_query')">
          <v-list-item-title>New Query</v-list-item-title>
        </v-list-item>
        <v-list-item v-if="contextMenu.node?.data.type === 'table'" @click="handleContextMenuAction('select_top_1000')">
          <v-list-item-title>Select Top 1000 Rows</v-list-item-title>
        </v-list-item>
      </v-list>
    </v-menu>

    <v-alert v-if="explorerStore.error" type="error" density="compact" class="mt-2">
      {{ explorerStore.error }}
    </v-alert>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useExplorerStore, type ExplorerNode } from '@/stores/explorer'
import { useConnectionStore } from '@/stores/connection'
import { useTabsStore } from '@/stores/tabs'
import { useNavigationStore } from '@/stores/navigation'
import AppTreeview from './AppTreeview.vue'

const explorerStore = useExplorerStore()
const connectionStore = useConnectionStore()
const tabsStore = useTabsStore()
const navigationStore = useNavigationStore()

const { selectedExplorerConnectionId } = storeToRefs(navigationStore)
const { activeConnections } = storeToRefs(connectionStore)

const treeviewNodes = computed<ExplorerNode[]>(() => {
  const connectionId = selectedExplorerConnectionId.value
  if (!connectionId) return []

  const connection = activeConnections.value[connectionId]
  if (!connection) return []

  return [
    {
      key: `conn-${connection.id}`,
      label: connection.name,
      icon: 'mdi-server',
      children: explorerStore.nodes,
      data: { type: 'connection', connectionId: connection.id },
    }
  ]
})

const contextMenu = reactive({
  visible: false,
  x: 0,
  y: 0,
  node: null as ExplorerNode | null,
})

// Fetch databases when the selected connection for the explorer changes
watch(selectedExplorerConnectionId, (newId) => {
  if (newId) {
    explorerStore.fetchDatabases(newId)
  } else {
    explorerStore.clearExplorer()
  }
}, { immediate: true })

function onExpand(node: ExplorerNode) {
  explorerStore.expandNode(node)
}

function onNodeClick(_node: ExplorerNode) {
  // Could be used to preview table data, etc. in the future
}

function onContextMenu({ event, node }: { event: MouseEvent, node: ExplorerNode }) {
  contextMenu.visible = false // Hide first to recalculate position
  contextMenu.x = event.clientX
  contextMenu.y = event.clientY
  contextMenu.node = node
  contextMenu.visible = true
}

function handleContextMenuAction(action: 'new_query' | 'select_top_1000') {
  const node = contextMenu.node
  if (!node) return

  const connectionId = node.data.connectionId
  if (!connectionId) return
  
  if (action === 'new_query') {
    tabsStore.addTab(connectionId)
  } else if (action === 'select_top_1000' && node.data.type === 'table') {
    const { db, schema, table } = node.data
    let query = ''
    const activeConnection = connectionStore.activeConnections[connectionId]
    if (activeConnection?.dbType === 'mysql') {
      query = `SELECT * FROM \`${db}\`.\`${table}\` LIMIT 1000;`
    } else if (activeConnection?.dbType === 'postgres') {
      query = `SELECT * FROM "${schema}"."${table}" LIMIT 1000;`
    } else {
      // Generic fallback for other SQL dialects
      query = `SELECT * FROM ${table} LIMIT 1000;`
    }
    tabsStore.addTab(connectionId, query)
  }

  contextMenu.visible = false
}
</script>
