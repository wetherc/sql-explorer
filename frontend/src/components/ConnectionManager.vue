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
          >
            <template v-slot:append>
              <v-btn
                icon="mdi-connection"
                variant="text"
                @click="connectToDatabase(conn)"
                color="success"
              ></v-btn>
              <v-btn
                icon="mdi-pencil"
                variant="text"
                @click="editConnection(conn)"
                color="info"
              ></v-btn>
              <v-btn
                icon="mdi-delete"
                variant="text"
                @click="deleteConnection(conn.id)"
                color="error"
              ></v-btn>
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
  </v-container>
</template>

<script setup lang="ts">
import { onMounted } from 'vue'
import { storeToRefs } from 'pinia'
import { useConnectionManagerStore, type SavedConnection } from '@/stores/connectionManager'
import ConnectionForm from './ConnectionForm.vue'
import { invoke } from '@tauri-apps/api/tauri'

const connectionManagerStore = useConnectionManagerStore()
const { connections, loading, error, showConnectionForm } = storeToRefs(connectionManagerStore)
const { fetchConnections, newConnection, editConnection, deleteConnection } = connectionManagerStore

onMounted(() => {
  fetchConnections()
})

async function connectToDatabase(connection: SavedConnection) {
  // Construct connection string based on dbType and fields
  let connectionString = ''
  switch (connection.dbType) {
    case 'Mssql':
      connectionString = `server=${connection.host};port=${connection.port};user=${connection.user};database=${connection.database};TrustServerCertificate=true;`
      break
    case 'Mysql':
      connectionString = `mysql://${connection.user}@${connection.host}:${connection.port}/${connection.database}`
      break
    case 'Postgres':
      connectionString = `postgresql://${connection.user}@${connection.host}:${connection.port}/${connection.database}`
      break
  }

  try {
    // Call the backend connect command
    await invoke('connect', { connectionString, dbType: connection.dbType })
    console.log('Connected to database:', connection.name)
    // TODO: Update main connection store, switch to explorer view
  } catch (e: any) {
    console.error('Failed to connect:', e)
    // TODO: Show error to user
  }
}
</script>
