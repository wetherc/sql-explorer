<script setup lang="ts">
import { onMounted, ref, reactive, computed } from 'vue'
import { useExplorerStore } from '@/stores/explorer'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { DbType } from '@/types/savedConnection'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'

interface TreeNode {
  name: string
  type: 'database' | 'folder' | 'schema' | 'table'
  expanded: boolean
  children?: TreeNode[]
  // Store context for nested nodes
  parentInfo?: {
    db?: string
    schema?: string
  }
}

const explorerStore = useExplorerStore()
const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()

const tree = ref<TreeNode[]>([])
const contextMenu = reactive({
  visible: false,
  x: 0,
  y: 0,
  items: [] as MenuItem[],
  node: null as TreeNode | null
})

const isMySql = computed(() => connectionStore.dbType === DbType.Mysql)

onMounted(async () => {
  await explorerStore.fetchDatabases()
  tree.value = explorerStore.databases.map((db) => ({
    name: db.name,
    type: 'database',
    children: isMySql.value
      ? [{ name: 'Tables', type: 'folder', children: [], parentInfo: { db: db.name }, expanded: false }]
      : [{ name: 'Schemas', type: 'folder', children: [], parentInfo: { db: db.name }, expanded: false }],
    expanded: false,
    parentInfo: { db: db.name }
  }))
})

async function toggleNode(node: TreeNode) {
  node.expanded = !node.expanded
  if (!node.expanded) return

  const parentInfo = node.parentInfo
  if (!parentInfo) return

  try {
    if (isMySql.value) {
      if (node.type === 'folder' && node.name === 'Tables' && parentInfo.db) {
        await explorerStore.fetchTables(parentInfo.db)
        node.children = explorerStore.tables.map((t) => ({
          name: t.TABLE_NAME,
          type: 'table',
          parentInfo: { db: parentInfo.db, schema: parentInfo.db }, // In MySQL, schema is the same as db
          expanded: false
        }))
      }
    } else {
      // MS SQL logic
      if (node.type === 'folder' && node.name === 'Schemas' && parentInfo.db) {
        await explorerStore.fetchSchemas() // This should be adapted to take db name
        node.children = explorerStore.schemas.map((s) => ({
          name: s.name,
          type: 'schema',
          children: [
            {
              name: 'Tables',
              type: 'folder',
              children: [],
              parentInfo: { db: parentInfo.db, schema: s.name },
              expanded: false
            }
          ],
          expanded: false,
          parentInfo: { db: parentInfo.db, schema: s.name }
        }))
      } else if (node.type === 'folder' && node.name === 'Tables' && parentInfo.schema) {
        await explorerStore.fetchTables(parentInfo.schema)
        node.children = explorerStore.tables.map((t) => ({
          name: t.TABLE_NAME,
          type: 'table',
          parentInfo: { db: parentInfo.db, schema: parentInfo.schema },
          expanded: false
        }))
      }
    }
  } catch (error) {
    console.error('Failed to toggle node:', error)
    // You might want to display this error to the user
  }
}

function showContextMenu(event: MouseEvent, node: TreeNode) {
  contextMenu.visible = true
  contextMenu.x = event.clientX
  contextMenu.y = event.clientY
  contextMenu.node = node

  switch (node.type) {
    case 'database':
      contextMenu.items = [{ label: 'New Query', action: 'new-query' }]
      break
    case 'table':
      contextMenu.items = [
        { label: 'New Query', action: 'new-query' },
        { label: 'Select Top 1000 Rows', action: 'select-top-1000' }
      ]
      break
    default:
      contextMenu.items = []
      break
  }
}

function closeContextMenu() {
  contextMenu.visible = false
}

function handleContextMenuAction(action: string) {
  const node = contextMenu.node
  if (!node) return

  switch (action) {
    case 'new-query':
      tabsStore.addTab('') // Add a new blank tab
      break
    case 'select-top-1000':
      if (node.type === 'table' && node.parentInfo) {
        const schema = node.parentInfo.schema
        const db = node.parentInfo.db
        if (schema && db) {
          const query = isMySql.value
            ? `SELECT * FROM \`${db}\`.\`${node.name}\` LIMIT 1000;`
            : `SELECT TOP (1000) * FROM [${schema}].[${node.name}]`
          tabsStore.addTab(query)
        }
      }
      break
  }
}
</script>

<template>
  <div class="db-explorer">
    <h3>Database Explorer</h3>
    <div v-if="explorerStore.loading">Loading...</div>
    <div v-else-if="explorerStore.error" class="error-message">
      {{ explorerStore.error }}
    </div>
    <div class="tree-view" v-else>
      <ul>
        <li v-for="node in tree" :key="node.name">
          <span class="tree-item" @click="toggleNode(node)" @contextmenu.prevent="showContextMenu($event, node)">
            {{ node.expanded ? '▼' : '►' }} {{ node.name }}
          </span>
          <ul v-if="node.expanded && node.children">
            <li v-for="child in node.children" :key="child.name">
              <span class="tree-item" @click="toggleNode(child)" @contextmenu.prevent="showContextMenu($event, child)">
                <span v-if="child.children">{{ child.expanded ? '▼' : '►' }}</span>
                {{ child.name }}
              </span>
              <ul v-if="child.expanded && child.children">
                <li v-for="grandchild in child.children" :key="grandchild.name">
                  <span class="tree-item" @click="toggleNode(grandchild)" @contextmenu.prevent="showContextMenu($event, grandchild)">
                    <span v-if="grandchild.children">{{ grandchild.expanded ? '▼' : '►' }}</span>
                    {{ grandchild.name }}
                  </span>
                  <ul v-if="grandchild.expanded && grandchild.children">
                    <li v-for="table in grandchild.children" :key="table.name">
                        <span class="tree-item" @click="toggleNode(table)" @contextmenu.prevent="showContextMenu($event, table)">
                            {{ table.expanded ? '▼' : '►' }} {{ table.name }}
                        </span>
                    </li>
                  </ul>
                </li>
              </ul>
            </li>
          </ul>
        </li>
      </ul>
    </div>
    <ContextMenu
      v-if="contextMenu.visible"
      :items="contextMenu.items"
      :x="contextMenu.x"
      :y="contextMenu.y"
      @close="closeContextMenu"
      @select="handleContextMenuAction"
    />
  </div>
</template>

<style scoped>
.db-explorer {
  padding: 10px;
  background-color: #f8f9fa;
  height: 100%;
  overflow-y: auto;
  position: relative;
}

h3 {
  color: #343a40;
  margin-bottom: 15px;
}

.tree-view ul {
  list-style-type: none;
  padding-left: 20px;
  margin: 0;
}

.tree-view ul li {
  margin-top: 5px;
}

.tree-item {
  cursor: pointer;
  padding: 2px 5px;
  border-radius: 3px;
}

.tree-item:hover {
  background-color: #e2e6ea;
}

.error-message {
  color: red;
}
</style>
