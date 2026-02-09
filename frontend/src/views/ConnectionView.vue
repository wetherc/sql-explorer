<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import {
  buildConnectionString,
  type AuthType,
} from '@/utils/connectionStringBuilder'

const connectionStore = useConnectionStore()

const server = ref('localhost')
const port = ref<number | undefined>(1433)
const database = ref('master')
const applicationName = ref('sql-explorer')
const connectTimeout = ref(15)
const username = ref('sa')
const password = ref('Password123')
const authType = ref<AuthType>('sql')
const encrypt = ref<'false' | 'true'>('true')
const trustServerCertificate = ref(true)

const isSqlAuth = computed(() => authType.value === 'sql')
const isNamedInstance = computed(() => server.value.includes('\\'))

watch(isNamedInstance, (isNamed) => {
  if (isNamed) {
    port.value = undefined
  } else {
    port.value = 1433
  }
})

async function handleConnect() {
  try {
    const connectionString = buildConnectionString({
      server: server.value,
      port: port.value,
      database: database.value,
      applicationName: applicationName.value,
      connectTimeout: connectTimeout.value,
      authType: authType.value,
      username: authType.value === 'sql' ? username.value : undefined,
      password: authType.value === 'sql' ? password.value : undefined,
      encrypt: encrypt.value,
      trustServerCertificate: trustServerCertificate.value,
    })
    await connectionStore.connect(connectionString)
  } catch (error: any) {
    connectionStore.errorMessage = error.message
  }
}
</script>

<template>
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
          <option value="sql">SQL Server Authentication</option>
          <option value="integrated">Microsoft Entra / Integrated</option>
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
            required
            :disabled="connectionStore.isConnecting"
          />
        </div>
      </template>

      <button type="submit" :disabled="connectionStore.isConnecting">
        {{ connectionStore.isConnecting ? 'Connecting...' : 'Connect' }}
      </button>
      <p v-if="connectionStore.errorMessage" class="error-message">
        {{ connectionStore.errorMessage }}
      </p>
    </form>
  </div>
</template>

<style scoped>
.connection-view {
  display: flex;
  justify-content: center;
  align-items: center;
  height: 100vh;
  padding: 20px;
}

form {
  width: 100%;
  max-width: 500px;
  padding: 2rem;
  border: 1px solid #ccc;
  border-radius: 8px;
  background-color: #f9f9f9;
  overflow-y: auto;
  max-height: 90vh;
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

button {
  width: 100%;
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
</style>
