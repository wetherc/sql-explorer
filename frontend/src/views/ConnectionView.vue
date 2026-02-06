<script setup lang="ts">
import { ref } from 'vue'

const connectionString = ref('')
const emit = defineEmits(['connect'])

function handleConnect() {
  if (connectionString.value) {
    emit('connect', connectionString.value)
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
        />
      </div>
      <button type="submit">Connect</button>
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
</style>
