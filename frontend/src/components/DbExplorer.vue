<script setup lang="ts">
import { onMounted, ref, computed } from 'vue'
import { useExplorerStore } from '@/stores/explorer'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { DbType } from '@/types/savedConnection'
import Tree from 'primevue/tree'
import { type TreeNode } from 'primevue/treenode'
import ContextMenu from 'primevue/contextmenu'
// import Skeleton from 'primevue/skeleton'

const explorerStore = useExplorerStore()
const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()

const nodes = computed(() => explorerStore.nodes)
const contextMenu = ref<InstanceType<typeof ContextMenu> | null>(null)
const selectedNode = ref<TreeNode | null>(null)
const selectedNodeKey = ref<string | null>(null) // Added for selectionKeys

const isMySql = computed(() => connectionStore.dbType === DbType.Mysql)
const isPostgres = computed(() => connectionStore.dbType === DbType.Postgres)

const contextMenuItems = computed(() => {
  if (!selectedNode.value) return []
  const nodeType = selectedNode.value.data.type
  const items = [{ label: 'New Query', icon: 'pi pi-fw pi-plus', command: () => tabsStore.addTab() }]
  if (nodeType === 'table') {
    items.push({
      label: 'Select Top 1000 Rows',
      icon: 'pi pi-fw pi-table',
      command: handleSelectTop1000,
    })
  }
  return items
})

onMounted(async () => {
  await explorerStore.fetchDatabases()
})

async function onNodeExpand(node: TreeNode) {
  // console.log('Node expanded:', node.label)
}

function onNodeSelect(event: { node: TreeNode }) {
  selectedNode.value = event.node
  const node = event.node
  if (node.data.type === 'table') {
    const { db, schema, table } = node.data
    if (db && schema && table) {
      let query = ''
      if (isMySql.value) {
        query = `SELECT * FROM \`${db}\`.\`${table}\` LIMIT 1000;`
      } else if (isPostgres.value) {
        query = `SELECT * FROM "${schema}"."${table}" LIMIT 1000;`
      } else {
        query = `SELECT TOP (1000) * FROM [${schema}].[${table}]`
      }
      tabsStore.addTab(query)
    }
  }
}

function onNodeContextMenu(event: { originalEvent: MouseEvent; node: TreeNode }) {
  event.originalEvent.preventDefault() // Prevent default browser context menu
  selectedNode.value = event.node
  contextMenu.value?.show(event.originalEvent)
}

function handleSelectTop1000() {
  if (!selectedNode.value) return
  const node = selectedNode.value
  if (node.data.type === 'table') {
    const { db, schema, table } = node.data
    if (db && schema && table) {
      let query = ''
      if (isMySql.value) {
        query = `SELECT * FROM \`${db}\`.\`${table}\` LIMIT 1000;`
      } else if (isPostgres.value) {
        query = `SELECT * FROM "${schema}"."${table}" LIMIT 1000;`
      } else {
        query = `SELECT TOP (1000) * FROM [${schema}].[${table}]`
      }
      tabsStore.addTab(query)
    }
  }
}
</script>

<template>
  <div class="db-explorer">
    <div>
      <ContextMenu ref="contextMenu" :model="contextMenuItems" />
      <Tree
        :value="nodes"
        @node-expand="onNodeExpand"
        @node-contextmenu="onNodeContextMenu"
        @node-select="onNodeSelect"
        :loading="false"
        selectionMode="single"
        v-model:selectionKeys="selectedNodeKey"
        class="w-full md:w-30rem"
      >
        <template #default="slotProps">
          <b>{{ slotProps.node.label }}</b>
        </template>
      </Tree>
    </div>
  </div>
</template>

<style scoped>
.db-explorer {
  height: 100%;
  overflow-y: auto;
}

:deep(.p-tree) {
  border: none;
  padding: 0;
  background: transparent;
}
</style>
