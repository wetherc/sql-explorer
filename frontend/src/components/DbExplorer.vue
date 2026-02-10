<script setup lang="ts">
import { onMounted, ref, computed } from 'vue'
import { useExplorerStore } from '@/stores/explorer'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { DbType } from '@/types/savedConnection'
import Tree, { type TreeNode } from 'primevue/tree'
import ContextMenu from 'primevue/contextmenu'
import Skeleton from 'primevue/skeleton'

const explorerStore = useExplorerStore()
const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()

const nodes = ref<TreeNode[]>([])
const contextMenu = ref<ContextMenu | null>(null)
const selectedNode = ref<TreeNode | null>(null)

const isMySql = computed(() => connectionStore.dbType === DbType.Mysql)
const isPostgres = computed(() => connectionStore.dbType === DbType.Postgres)

const contextMenuItems = computed(() => {
  if (!selectedNode.value) return []
  const nodeType = selectedNode.value.data.type
  const items = [{ label: 'New Query', icon: 'pi pi-fw pi-plus', command: () => tabsStore.addTab('') }]
  if (nodeType === 'table') {
    items.push({
      label: 'Select Top 1000 Rows',
      icon: 'pi pi-fw pi-table',
      command: () => handleSelectTop1000(),
    })
  }
  return items
})

onMounted(async () => {
  await explorerStore.fetchDatabases()
  nodes.value = explorerStore.databases.map((db) => ({
    key: db.name,
    label: db.name,
    icon: 'pi pi-fw pi-database',
    data: { type: 'database', db: db.name },
    children: [{ key: `${db.name}-loader` }], // Dummy node for lazy loading
  }))
})

async function onNodeExpand(node: TreeNode) {
  if (node.children?.[0]?.key.endsWith('-loader')) {
    const parentInfo = node.data
    node.children = [] // Clear dummy node

    try {
      if (isMySql.value) {
        if (parentInfo.type === 'database' && parentInfo.db) {
          await explorerStore.fetchTables(parentInfo.db)
          node.children = explorerStore.tables.map((t) => ({
            key: `${parentInfo.db}-${t.TABLE_NAME}`,
            label: t.TABLE_NAME,
            icon: 'pi pi-fw pi-table',
            data: { type: 'table', db: parentInfo.db, schema: parentInfo.db, table: t.TABLE_NAME },
          }))
        }
      } else { // MS SQL and PostgreSQL
        if (parentInfo.type === 'database' && parentInfo.db) {
          await explorerStore.fetchSchemas()
          node.children = explorerStore.schemas.map((s) => ({
            key: `${parentInfo.db}-${s.name}`,
            label: s.name,
            icon: 'pi pi-fw pi-folder',
            data: { type: 'schema', db: parentInfo.db, schema: s.name },
            children: [{ key: `${parentInfo.db}-${s.name}-loader` }],
          }))
        } else if (parentInfo.type === 'schema' && parentInfo.db && parentInfo.schema) {
          await explorerStore.fetchTables(parentInfo.schema)
          node.children = explorerStore.tables.map((t) => ({
            key: `${parentInfo.db}-${parentInfo.schema}-${t.TABLE_NAME}`,
            label: t.TABLE_NAME,
            icon: 'pi pi-fw pi-table',
            data: { type: 'table', db: parentInfo.db, schema: parentInfo.schema, table: t.TABLE_NAME },
          }))
        }
      }
    } catch (error) {
      console.error('Failed to expand node:', error)
      // Display error to user via toast or message
    }
  }
}

function onNodeContextMenu(event: { originalEvent: MouseEvent; node: TreeNode }) {
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
    <div v-if="explorerStore.loading && nodes.length === 0">
      <Skeleton height="2rem" class="mb-2" />
      <Skeleton height="2rem" class="mb-2" />
      <Skeleton height="2rem" class="mb-2" />
    </div>
    <div v-else-if="explorerStore.error" class="error-message">
      <p>Error loading databases:</p>
      <pre>{{ explorerStore.error }}</pre>
    </div>
    <div v-else>
      <ContextMenu ref="contextMenu" :model="contextMenuItems" />
      <Tree
        :value="nodes"
        @node-expand="onNodeExpand"
        @node-contextmenu="onNodeContextMenu"
        :loading="explorerStore.loading"
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
