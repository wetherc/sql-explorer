<template>
  <div>
    <v-progress-linear v-if="explorerStore.loading" indeterminate></v-progress-linear>
    <AppTreeview :nodes="nodes" @expand="onExpand" @node-click="onNodeClick" @contextmenu="onContextMenu" />

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
import { useExplorerStore, type ExplorerNode } from '@/stores/explorer'
import { useConnectionStore } from '@/stores/connection'
import { useTabsStore } from '@/stores/tabs'
import AppTreeview from './AppTreeview.vue'

const explorerStore = useExplorerStore()
const connectionStore = useConnectionStore()
const tabsStore = useTabsStore()

const nodes = computed(() => explorerStore.nodes)

const contextMenu = reactive({
  visible: false,
  x: 0,
  y: 0,
  node: null as ExplorerNode | null,
})

// Fetch databases when the connection is established
watch(() => connectionStore.isConnected, (isConnected) => {
  if (isConnected) {
    explorerStore.fetchDatabases()
  } else {
    explorerStore.clearExplorer()
  }
}, { immediate: true })

function onExpand(node: ExplorerNode) {
  explorerStore.expandNode(node)
}

function onNodeClick(node: ExplorerNode) {
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

  if (action === 'new_query') {
    tabsStore.addTab()
  } else if (action === 'select_top_1000' && node.data.type === 'table') {
    const { db, schema, table } = node.data
    let query = ''
    if (connectionStore.dbType === 'Mysql') {
      query = `SELECT * FROM \`${db}\`.\`${table}\` LIMIT 1000;`
    } else if (connectionStore.dbType === 'Postgres') {
      query = `SELECT * FROM "${schema}"."${table}" LIMIT 1000;`
    } else {
      query = `SELECT TOP (1000) * FROM [${schema}].[${table}];`
    }
    tabsStore.addTab(query)
  }

  contextMenu.visible = false
}
</script>
