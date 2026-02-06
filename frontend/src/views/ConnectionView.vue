<script setup lang="ts">
import { ref, computed } from 'vue'
import { useConnectionStore } from '@/stores/connection'

type AuthType = 'sql' | 'integrated'

const connectionStore = useConnectionStore()

const server = ref('localhost')
const database = ref('master')
const username = ref('sa')
const password = ref('Password123')
const authType = ref<AuthType>('sql')

const isSqlAuth = computed(() => authType.value === 'sql')

async function handleConnect() {
  // This is a temporary implementation. Task M2.3 will create a dedicated builder.
  let connectionString = `server=${server.value};database=${database.value};TrustServerCertificate=true`;
  if (isSqlAuth.value) {
    connectionString += `;user=${username.value};password=${password.value}`;
  } else {
    connectionString += ';Authentication=ActiveDirectoryIntegrated';
  }
  await connectionStore.connect(connectionString)
}
</script>

<template>
  <div class="connection-view">
    <form @submit.prevent="handleConnect">
      <h2>Connect to Database</h2>

      <div class="form-group">
        <label for="server">Server</label>
        <input
          id="server"
          v-model="server"
          type="text"
          placeholder="localhost"
          required
          :disabled="connectionStore.isConnecting"
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

input, select {
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
