<script setup lang="ts">
import { computed } from 'vue'
import MonacoEditor from 'monaco-editor-vue3'
import { type QueryResponse, type ResultSet } from '@/types/query'

const props = defineProps<{
  query: string,
  response?: QueryResponse | null,
  isLoading: boolean,
  errorMessage: string | null,
}>()

const emit = defineEmits<{
  (e: 'update:query', value: string): void,
  (e: 'execute-query'): void,
}>()

const localQuery = computed({
  get: () => props.query,
  set: (value) => emit('update:query', value),
})

const firstResultSet = computed<ResultSet | null>(() => {
  return props.response && props.response.results.length > 0
    ? props.response.results[0]
    : null
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

    <div v-if="errorMessage" class="error-panel">
      <h3>Error</h3>
      <pre>{{ errorMessage }}</pre>
    </div>

    <div v-if="response && response.messages.length > 0" class="messages-panel">
        <h3>Messages</h3>
        <pre>{{ response.messages.join('\n') }}</pre>
    </div>

    <div v-if="firstResultSet?.rows.length" class="results-panel">
      <h3>Results</h3>
      <table>
        <thead>
          <tr>
            <th v-for="col in firstResultSet.columns" :key="col">{{ col }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, rowIndex) in firstResultSet.rows" :key="rowIndex">
            <td v-for="col in firstResultSet.columns" :key="col">
              {{ row[col] }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <div v-else-if="!isLoading && response && !errorMessage && (!firstResultSet || firstResultSet.rows.length === 0)" class="results-panel">
        <p>Query executed successfully. No rows returned.</p>
    </div>
  </div>
</template>
