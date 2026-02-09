<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import { buildConnectionString } from '@/utils/connectionStringBuilder'
import type { SavedConnection } from '@/types/savedConnection'
import { AuthType } from '@/types/savedConnection'

import SavedConnections from '@/components/SavedConnections.vue'
import SaveConnectionDialog from '@/components/SaveConnectionDialog.vue'

const connectionStore = useConnectionStore()
const savedConnectionsStore = useSavedConnectionsStore()

const server = ref('localhost')
const port = ref<number | undefined>(1433)
const database = ref('master')
const applicationName = ref('sql-explorer')
const connectTimeout = ref(15)
const username = ref('sa')
const password = ref('')
const authType = ref<AuthType>(AuthType.Sql)
const encrypt = ref<'false' | 'true'>('true')
const trustServerCertificate = ref(true)

const showSaveDialog = ref(false)

const isSqlAuth = computed(() => authType.value === AuthType.Sql)
const isNamedInstance = computed(() => server.value.includes('\\'))

watch(isNamedInstance, (isNamed) => {
  if (isNamed) {
    port.value = undefined
  } else {
    port.value = 1433
  }
})

async function handleConnect() {
  let finalPassword = password.value
  // If we are using SQL auth and the password field is empty,
  // it could be a saved connection. Try to fetch the password.
  if (isSqlAuth.value && !password.value) {
    const matchingSavedConn = savedConnectionsStore.connections.find(
      (c) =>
        c.server === server.value &&
        c.database === database.value &&
        c.user === username.value
    )
    if (matchingSavedConn) {
      const savedPassword = await savedConnectionsStore.getPassword(matchingSavedConn.name)
      if (savedPassword) {
        finalPassword = savedPassword
      }
    }
  }

  try {
    const connectionString = buildConnectionString({
      server: server.value,
      port: port.value,
      database: database.value,
      applicationName: applicationName.value,
      connectTimeout: connectTimeout.value,
      authType: authType.value === AuthType.Sql ? 'sql' : 'integrated',
      username: isSqlAuth.value ? username.value : undefined,
      password: isSqlAuth.value ? finalPassword : undefined,
      encrypt: encrypt.value,
      trustServerCertificate: trustServerCertificate.value
    })
    await connectionStore.connect(connectionString)
  } catch (error: unknown) {
    connectionStore.errorMessage = error instanceof Error ? error.message : 'An unknown error occurred.'
  }
}

function handleSelectSavedConnection(connection: SavedConnection) {
  server.value = connection.server
  database.value = connection.database
  authType.value = connection.auth_type
  if (connection.user) {
    username.value = connection.user
  } else {
    username.value = ''
  }
  // Clear the password field. It will be fetched on connect if needed.
  password.value = ''
}

function openSaveDialog() {
  showSaveDialog.value = true
}

async function handleSaveConnection(name: string, passwordToSave?: string) {
  const connectionToSave: Omit<SavedConnection, 'name'> = {
    server: server.value,
    database: database.value,
    auth_type: authType.value,
    user: isSqlAuth.value ? username.value : undefined
  }
  try {
    await savedConnectionsStore.saveConnection({ name, ...connectionToSave }, passwordToSave)
  } catch (error) {
    alert(`Error saving connection: ${error}`)
  }
}
</script>

<template>
  <div class="connection-view-wrapper">
    <div class="connection-view">
      <form @submit.prevent="handleConnect">
        <h2>Connect to Database</h2>

        <div class="form-group">
          <label for="server">Server or Server\Instance</label>
          <input
            id="server"
            v-model="server"
            type="text"
            placeholder="localhost or localhost\\SQLEXPRESS"
            required
            :disabled="connectionStore.isConnecting"
          />
        </div>

        <div class="form-group">
          <label for="port">Port</label>
          <input
            id="port"
            v-model.number="port"
            type="number"
            placeholder="1433"
            :required="!isNamedInstance"
            :disabled="connectionStore.isConnecting || isNamedInstance"
          />
        </div>

        <div class="form-group">
          <label for="database">Database (optional)</label>
          <input
            id="database"
            v-model="database"
            type="text"
            placeholder="master"
            :disabled="connectionStore.isConnecting"
          />
        </div>

        <div class="form-group">
          <label for="appName">Application Name</label>
          <input
            id="appName"
            v-model="applicationName"
            type="text"
            :disabled="connectionStore.isConnecting"
          />
        </div>

        <div class="form-group">
          <label for="timeout">Connect Timeout (seconds)</label>
          <input
            id="timeout"
            v-model.number="connectTimeout"
            type="number"
            required
            :disabled="connectionStore.isConnecting"
          />
        </div>

        <div class="form-group">
          <label for="encrypt">Encryption</label>
          <select id="encrypt" v-model="encrypt" :disabled="connectionStore.isConnecting">
            <option value="true">On</option>
            <option value="false">Off</option>
          </select>
        </div>

        <div class="form-group">
          <label class="checkbox-label">
            <input
              type="checkbox"
              v-model="trustServerCertificate"
              :disabled="connectionStore.isConnecting"
            />
            Trust server certificate
          </label>
        </div>

        <div class="form-group">
          <label for="auth-type">Authentication</label>
          <select id="auth-type" v-model="authType" :disabled="connectionStore.isConnecting">
            <option :value="AuthType.Sql">SQL Server Authentication</option>
            <option :value="AuthType.Integrated">Microsoft Entra / Integrated</option>
          </select>
        </div>

        <template v-if="isSqlAuth">
          <div class="form-group">
            <label for="username">Username</label>
            <input
              id="username"
              v-model="username"
              type="text"
              placeholder="sa"
              required
              :disabled="connectionStore.isConnecting"
            />
          </div>

          <div class="form-group">
            <label for="password">Password</label>
            <input
              id="password"
              v-model="password"
              type="password"
              :disabled="connectionStore.isConnecting"
              placeholder="Leave blank for saved password"
            />
          </div>
        </template>

        <div class="form-actions">
          <button type="submit" :disabled="connectionStore.isConnecting">
            {{ connectionStore.isConnecting ? 'Connecting...' : 'Connect' }}
          </button>
          <button type="button" @click="openSaveDialog" :disabled="connectionStore.isConnecting">
            Save Connection
          </button>
        </div>
        <p v-if="connectionStore.errorMessage" class="error-message">
          {{ connectionStore.errorMessage }}
        </p>
      </form>
    </div>
    <div class="saved-connections-panel">
      <SavedConnections @select="handleSelectSavedConnection" />
    </div>

    <SaveConnectionDialog
      :show="showSaveDialog"
      :connection="{
        server,
        database,
        auth_type: authType,
        user: isSqlAuth ? username : undefined
      }"
      :password="isSqlAuth ? password : undefined"
      @close="showSaveDialog = false"
      @save="handleSaveConnection"
    />
  </div>
</template>

<style scoped>
.connection-view-wrapper {
  display: grid;
  grid-template-columns: 2fr 1fr;
  height: 100vh;
  gap: 20px;
  padding: 20px;
}
.connection-view {
  display: flex;
  justify-content: center;
  align-items: flex-start;
  overflow-y: auto;
}

form {
  width: 100%;
  max-width: 500px;
  padding: 2rem;
  border: 1px solid #ccc;
  border-radius: 8px;
  background-color: #f9f9f9;
}

h2 {
  text-align: center;
  margin-bottom: 1.5rem;
}

.form-group {
  margin-bottom: 1rem;
}

label {
  display: block;
  margin-bottom: 0.5rem;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

input[type='checkbox'] {
  width: auto;
}

input,
select {
  width: 100%;
  padding: 0.5rem;
  font-size: 1rem;
  border: 1px solid #ccc;
  border-radius: 4px;
  background-color: white;
}

.form-actions {
  display: flex;
  gap: 10px;
  margin-top: 1rem;
}

button {
  flex: 1;
  padding: 0.75rem;
  font-size: 1rem;
  color: #fff;
  background-color: #007bff;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  transition: background-color 0.2s;
}

button:hover {
  background-color: #0056b3;
}

button:disabled {
  background-color: #5a9ed8;
  cursor: not-allowed;
}

.error-message {
  color: #d8000c;
  background-color: #ffbaba;
  border: 1px solid #d8000c;
  border-radius: 4px;
  padding: 0.75rem;
  margin-top: 1rem;
  text-align: center;
}

.saved-connections-panel {
  border-left: 1px solid #ccc;
  padding-left: 20px;
  overflow-y: auto;
}
</style>
