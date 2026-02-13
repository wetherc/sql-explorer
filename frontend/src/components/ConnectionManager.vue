<template>
  <v-container>
    <v-row>
      <v-col cols="12">
        <v-btn color="primary" @click="newConnection" prepend-icon="mdi-plus">New Connection</v-btn>
      </v-col>
    </v-row>
    <v-row>
      <v-col cols="12">
        <v-list>
          <v-list-item
            v-for="conn in connections"
            :key="conn.id"
            :title="conn.name"
            :subtitle="conn.host"
            :active="isActive(conn)"
            @click="selectConnectionForExplorer(conn)"
          >
            <template v-slot:append>
              <v-tooltip location="top">
                <template v-slot:activator="{ props }">
                  <v-btn
                    v-bind="props"
                    :icon="isActive(conn) ? 'mdi-lan-disconnect' : 'mdi-lan-connect'"
                    variant="text"
                    :color="isActive(conn) ? 'error' : 'success'"
                    @click.stop="handleConnectionToggle(conn)"
                  ></v-btn>
                </template>
                <span>{{ isActive(conn) ? 'Disconnect' : 'Connect' }}</span>
              </v-tooltip>

              <v-tooltip location="top">
                <template v-slot:activator="{ props }">
                  <v-btn
                    v-bind="props"
                    icon="mdi-pencil"
                    variant="text"
                    @click.stop="editConnection(conn)"
                    color="info"
                  ></v-btn>
                </template>
                <span>Edit Connection</span>
              </v-tooltip>

              <v-tooltip location="top">
                <template v-slot:activator="{ props }">
                  <v-btn
                    v-bind="props"
                    icon="mdi-delete"
                    variant="text"
                    @click.stop="deleteConnection(conn.id)"
                    color="error"
                  ></v-btn>
                </template>
                <span>Delete Connection</span>
              </v-tooltip>
            </template>
          </v-list-item>
        </v-list>
        <div v-if="loading">Loading connections...</div>
        <div v-if="error">{{ error }}</div>
      </v-col>
    </v-row>

    <v-dialog v-model="showConnectionForm" persistent max-width="600px">
      <ConnectionForm />
    </v-dialog>

    <v-dialog v-model="disconnectDialog" persistent max-width="400px">
      <v-card>
        <v-card-title>Confirm Disconnect</v-card-title>
        <v-card-text>
          Are you sure you want to disconnect from <strong>{{ connectionToDisconnect?.name }}</strong>?
        </v-card-text>
        <v-card-actions>
          <v-spacer></v-spacer>
          <v-btn text @click="disconnectDialog = false">Cancel</v-btn>
          <v-btn color="error" @click="confirmDisconnect">Disconnect</v-btn>
        </v-card-actions>
      </v-card>
    </v-dialog>
  </v-container>
</template>

<script setup lang="ts">
import { onMounted, computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useConnectionManagerStore, type SavedConnection } from '@/stores/connectionManager'
import { useConnectionStore } from '@/stores/connection'
import { useNavigationStore } from '@/stores/navigation'
import ConnectionForm from './ConnectionForm.vue'

const connectionManagerStore = useConnectionManagerStore()
const connectionStore = useConnectionStore()
const navigationStore = useNavigationStore()

const { connections, loading, error, showConnectionForm } = storeToRefs(connectionManagerStore)
const { activeConnections } = storeToRefs(connectionStore)
const { fetchConnections, newConnection, editConnection, deleteConnection } = connectionManagerStore

const disconnectDialog = ref(false)
const connectionToDisconnect = ref<SavedConnection | null>(null)

const isActive = (conn: SavedConnection) => {
  return !!activeConnections.value[conn.id]
}

onMounted(() => {
  fetchConnections()
})

function handleConnectionToggle(connection: SavedConnection) {
  if (isActive(connection)) {
    promptToDisconnect(connection)
  } else {
    connectToDatabase(connection)
  }
}

function selectConnectionForExplorer(connection: SavedConnection) {
  if (isActive(connection)) {
    navigationStore.setSelectedExplorerConnectionId(connection.id)
    navigationStore.setActiveView('explorer')
  }
}

function promptToDisconnect(connection: SavedConnection) {
  connectionToDisconnect.value = connection
  disconnectDialog.value = true
}

function confirmDisconnect() {
  if (connectionToDisconnect.value) {
    connectionStore.disconnect(connectionToDisconnect.value.id)
    // If the disconnected connection was the one being explored, unset it
    if (navigationStore.selectedExplorerConnectionId === connectionToDisconnect.value.id) {
      navigationStore.setSelectedExplorerConnectionId(null)
    }
  }
  disconnectDialog.value = false
  connectionToDisconnect.value = null
  navigationStore.setActiveView('connections')
}

async function connectToDatabase(connection: SavedConnection) {
  try {
    await connectionStore.connect(connection)
    navigationStore.setSelectedExplorerConnectionId(connection.id)
    navigationStore.setActiveView('explorer')
  } catch (e: any) {
    console.error('Failed to connect:', e)
    // TODO: Show error to user
  }
}
</script>
