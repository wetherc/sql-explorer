<script setup lang="ts">
import { onMounted } from 'vue'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import type { SavedConnection } from '@/types/savedConnection'

const store = useSavedConnectionsStore()

const emit = defineEmits<{
  (e: 'select', connection: SavedConnection): void
}>()

onMounted(() => {
  store.fetchConnections()
})

function onSelect(connection: SavedConnection) {
  emit('select', connection)
}

function onDelete(name: string) {
  if (confirm(`Are you sure you want to delete the connection "${name}"?`)) {
    store.deleteConnection(name)
  }
}
</script>

<template>
  <div class="saved-connections">
    <h3>Saved Connections</h3>
    <div v-if="store.isLoading">Loading...</div>
    <div v-else-if="store.errorMessage" class="error-message">
      {{ store.errorMessage }}
    </div>
    <ul v-else-if="store.connections.length">
      <li v-for="conn in store.connections" :key="conn.name">
        <span class="connection-name" @click="onSelect(conn)">
          {{ conn.name }} ({{ conn.server }})
        </span>
        <button @click="onDelete(conn.name)" class="delete-btn">Delete</button>
      </li>
    </ul>
    <div v-else>No saved connections yet.</div>
  </div>
</template>

<style scoped>
.saved-connections {
  margin-top: 20px;
}
ul {
  list-style-type: none;
  padding: 0;
}
li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px;
  border-bottom: 1px solid #eee;
  cursor: pointer;
}
li:hover {
  background-color: #f5f5f5;
}
.connection-name {
  flex-grow: 1;
}
.delete-btn {
  background: none;
  border: 1px solid #ff4d4d;
  color: #ff4d4d;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.8em;
  padding: 4px 8px;
}
.delete-btn:hover {
  background: #ff4d4d;
  color: white;
}
.error-message {
  color: red;
}
</style>
