<script setup lang="ts">
import { onMounted, ref, reactive } from 'vue';
import { useExplorerStore } from '@/stores/explorer';
import { useTabsStore } from '@/stores/tabs';
import ContextMenu, { type MenuItem } from './ContextMenu.vue';

const explorerStore = useExplorerStore();
const tabsStore = useTabsStore();

// Local state for the tree structure
const tree = ref<any[]>([]);

// Context menu state
const contextMenu = reactive({
  visible: false,
  x: 0,
  y: 0,
  items: [] as MenuItem[],
  node: null as any,
});

onMounted(async () => {
  await explorerStore.fetchDatabases();
  tree.value = explorerStore.databases.map(db => ({
    name: db.name,
    type: 'database',
    children: [
      { name: 'Schemas', type: 'folder', children: [], parent: db },
      // Other folders like 'Users' can be added here
    ],
    expanded: false,
  }));
});

async function toggleNode(node: any) {
  node.expanded = !node.expanded;
  if (node.expanded) {
    if (node.type === 'folder' && node.name === 'Schemas') {
      await explorerStore.fetchSchemas();
      node.children = explorerStore.schemas.map(s => ({
        name: s.name,
        type: 'schema',
        children: [
          { name: 'Tables', type: 'folder', children: [], parent: { database: node.parent, schema: s } },
        ],
        expanded: false,
      }));
    } else if (node.type === 'folder' && node.name === 'Tables') {
        await explorerStore.fetchTables(node.parent.schema.name);
        node.children = explorerStore.tables.map(t => ({
            name: t.TABLE_NAME,
            type: 'table',
            parent: node.parent,
            expanded: false,
        }));
    }
  }
}


function showContextMenu(event: MouseEvent, node: any) {
  contextMenu.visible = true;
  contextMenu.x = event.clientX;
  contextMenu.y = event.clientY;
  contextMenu.node = node;

  switch (node.type) {
    case 'database':
      contextMenu.items = [{ label: 'New Query', action: 'new-query' }];
      break;
    case 'table':
      contextMenu.items = [
        { label: 'New Query', action: 'new-query' },
        { label: 'Select Top 1000 Rows', action: 'select-top-1000' },
      ];
      break;
    default:
      contextMenu.items = [];
      break;
  }
}

function closeContextMenu() {
  contextMenu.visible = false;
}

function handleContextMenuAction(action: string) {
  const node = contextMenu.node;
  if (!node) return;

  switch (action) {
    case 'new-query':
      tabsStore.addTab(''); // Add a new blank tab
      break;
    case 'select-top-1000':
      if (node.type === 'table') {
        const query = `SELECT TOP (1000) * FROM [${node.parent.schema.name}].[${node.name}]`;
        tabsStore.addTab(query);
      }
      break;
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
