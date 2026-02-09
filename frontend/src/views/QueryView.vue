<script setup lang="ts">
import { ref } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import { useQueryStore } from '@/stores/query'
import MonacoEditor from 'monaco-editor-vue3'

const connectionStore = useConnectionStore()
const queryStore = useQueryStore()

const query = ref('SELECT 1')

async function handleExecute() {
  if (query.value) {
    await queryStore.executeQuery(query.value)
  }
}
</script>

<template>
  <div class="query-view">
    <header>
      <h1>SQL Explorer</h1>
      <button @click="connectionStore.disconnect()">Disconnect</button>
    </header>

    <div class="editor-panel">
      <form @submit.prevent="handleExecute">
        <div class="editor-container">
          <MonacoEditor
            v-model="query"
            theme="vs-dark"
            lang="sql"
            :options="{
              automaticLayout: true,
              minimap: { enabled: false },
              wordWrap: 'on',
            }"
          />
        </div>
        <button type="submit" :disabled="queryStore.isLoading">
          {{ queryStore.isLoading ? 'Executing...' : 'Execute' }}
        </button>
      </form>
    </div>

    <div v-if="queryStore.errorMessage" class="error-panel">
      <h3>Error</h3>
      <pre>{{ queryStore.errorMessage }}</pre>
    </div>

    <div v-if="queryStore.resultRows.length > 0" class="results-panel">
      <h3>Results</h3>
      <table>
        <thead>
          <tr>
            <th v-for="col in queryStore.resultColumns" :key="col">{{ col }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, rowIndex) in queryStore.resultRows" :key="rowIndex">
            <td v-for="col in queryStore.resultColumns" :key="col">
              {{ row[col] }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <div v-else-if="!queryStore.isLoading && queryStore.results" class="results-panel">
        <p>Query executed successfully. No rows returned.</p>
    </div>
  </div>
</template>

<style scoped>
.query-view {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden; /* Prevent body scrolling */
}

header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.5rem 1rem;
  background-color: #343a40;
  color: white;
  flex-shrink: 0;
}

header h1 {
  margin: 0;
  font-size: 1.25rem;
}

.editor-panel {
  padding: 1rem;
  border-bottom: 1px solid #ccc;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.editor-container {
  height: 200px; /* Or any desired height */
  border: 1px solid #ccc;
  border-radius: 4px;
  margin-bottom: 0.5rem;
  overflow: hidden; /* Ensures the editor respects the border-radius */
}

button {
  padding: 0.5rem 1rem;
  font-size: 1rem;
  color: #fff;
  background-color: #007bff;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  align-self: flex-start;
}

button:disabled {
  background-color: #5a9ed8;
  cursor: not-allowed;
}

.error-panel, .results-panel {
  padding: 1rem;
  overflow-y: auto;
  flex-grow: 1;
}

.error-panel pre {
  color: #d8000c;
  background-color: #ffbaba;
  padding: 1rem;
  border-radius: 4px;
  white-space: pre-wrap;
  word-wrap: break-word;
}

table {
  width: 100%;
  border-collapse: collapse;
}

th, td {
  border: 1px solid #ddd;
  padding: 8px;
  text-align: left;
}

thead {
  background-color: #f2f2f2;
}
</style>
