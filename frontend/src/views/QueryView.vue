<script setup lang="ts">
import { computed } from 'vue'
import MonacoEditor from 'monaco-editor-vue3'
import { QueryResult } from '@/stores/tabs' // Import QueryResult interface

const props = defineProps<{
  query: string,
  results: QueryResult,
  isLoading: boolean,
}>()

const emit = defineEmits<{
  (e: 'update:query', value: string): void,
  (e: 'execute-query'): void,
}>()

const localQuery = computed({
  get: () => props.query,
  set: (value) => emit('update:query', value),
})

function handleExecute() {
  emit('execute-query')
}
</script>

<template>
  <div class="query-view">
    <div class="editor-panel">
      <form @submit.prevent="handleExecute">
        <div class="editor-container">
          <MonacoEditor
            v-model="localQuery"
            theme="vs-dark"
            lang="sql"
            :options="{
              automaticLayout: true,
              minimap: { enabled: false },
              wordWrap: 'on',
            }"
          />
        </div>
        <button type="submit" :disabled="isLoading">
          {{ isLoading ? 'Executing...' : 'Execute' }}
        </button>
      </form>
    </div>

    <div v-if="results.errorMessage" class="error-panel">
      <h3>Error</h3>
      <pre>{{ results.errorMessage }}</pre>
    </div>

    <div v-if="results.rows.length > 0" class="results-panel">
      <h3>Results</h3>
      <table>
        <thead>
          <tr>
            <th v-for="col in results.columns" :key="col">{{ col }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, rowIndex) in results.rows" :key="rowIndex">
            <td v-for="col in results.columns" :key="col">
              {{ row[col] }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <div v-else-if="!isLoading && results" class="results-panel">
        <p>Query executed successfully. No rows returned.</p>
    </div>
  </div>
</template>

<style scoped>
.query-view {
  display: flex;
  flex-direction: column;
  height: 100%; /* Changed from 100vh to 100% to fit parent */
  overflow: hidden;
}

/* Removed header styles as it's no longer in this component */

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
