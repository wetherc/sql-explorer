<script setup lang="ts">
import { ref } from 'vue'
import { useConnectionStore } from '@/stores/connection'

const connectionStore = useConnectionStore()
const connectionString = ref('server=localhost;user=sa;password=Password123;database=master;TrustServerCertificate=true')

async function handleConnect() {
  if (connectionString.value) {
    await connectionStore.connect(connectionString.value)
  }
}
</script>

<template>
  <div class="connection-view">
    <form @submit.prevent="handleConnect">
      <h2>Connect to Database</h2>
      <div class="form-group">
        <label for="connection-string">Connection String</label>
        <input
          id="connection-string"
          v-model="connectionString"
          type="text"
          placeholder="server=...;user=...;password=..."
          required
          :disabled="connectionStore.isConnecting"
        />
      </div>
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

input {
  width: 100%;
  padding: 0.5rem;
  font-size: 1rem;
  border: 1px solid #ccc;
  border-radius: 4px;
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
