<script setup lang="ts">
import { computed, ref } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import { DbType } from '@/types/savedConnection'
import MonacoEditor from 'monaco-editor-vue3'
import { type QueryResponse, type ResultSet } from '@/types/query'
import { exportToCsv, downloadCsv } from '../utils/csvExporter'

import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import Button from 'primevue/button'
import TabView from 'primevue/tabview'
import TabPanel from 'primevue/tabpanel'
import Message from 'primevue/message'
import ProgressSpinner from 'primevue/progressspinner'

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

const connectionStore = useConnectionStore()

const localQuery = computed({
  get: () => props.query,
  set: (value) => emit('update:query', value),
})

const editorLang = computed(() => {
  switch (connectionStore.dbType) {
    case DbType.Mysql:
      return 'mysql'
    case DbType.Postgres:
      return 'pgsql'
    case DbType.Mssql:
    default:
      return 'sql'
  }
})

const activeResultTabIndex = ref(0); // For multiple result sets tabview

function handleExecute() {
  emit('execute-query')
}

function handleExportCsv(resultSet: ResultSet, index: number) {
  const filename = `query_results_${index + 1}_${new Date().toISOString().slice(0, 10)}.csv`;
  const csv = exportToCsv(resultSet.rows);
  downloadCsv(csv, filename);
}
</script>

<template>
  <div class="query-view flex flex-column h-full">
    <div class="editor-panel flex-none p-fluid">
      <form @submit.prevent="handleExecute" class="flex flex-column">
        <div class="editor-container">
          <MonacoEditor
            v-model="localQuery"
            theme="vs-dark"
            :lang="editorLang"
            :options="{
              automaticLayout: true,
              minimap: { enabled: false },
              wordWrap: 'on',
            }"
            class="h-15rem"
          />
        </div>
        <Button type="submit" :loading="isLoading" icon="pi pi-play" label="Execute Query" class="mt-2" />
      </form>
    </div>

    <div class="results-area flex-grow-1 overflow-auto mt-3 p-fluid relative">
      <ProgressSpinner v-if="isLoading" class="absolute top-0 left-0 w-full h-full flex align-items-center justify-content-center bg-black-alpha-50" />
      <Message v-if="errorMessage" severity="error" :closable="false">{{ errorMessage }}</Message>

      <div v-if="response && (response.results.length > 0 || response.messages.length > 0)" class="h-full">
        <TabView v-model:activeIndex="activeResultTabIndex" class="h-full">
          <TabPanel v-for="(resultSet, index) in response.results" :key="index" :value="`result-${index}`" :header="`Result ${index + 1}`" class="h-full flex flex-column">
            <div class="results-panel flex-grow-1 overflow-auto">
              <div class="results-header flex justify-content-between align-items-center mb-2">
                <h4>Result Set {{ index + 1 }} ({{ resultSet.rows.length }} rows)</h4>
                <Button icon="pi pi-download" label="Export to CSV" @click="handleExportCsv(resultSet, index)" class="p-button-sm p-button-outlined" />
              </div>
              <DataTable
                :value="resultSet.rows"
                stripedRows
                removableSort
                showGridlines
                scrollable
                scrollHeight="flex"
                class="p-datatable-sm w-full h-full"
              >
                <Column v-for="col in resultSet.columns" :field="col" :header="col" :key="col" sortable></Column>
              </DataTable>
            </div>
          </TabPanel>

          <TabPanel v-if="response.messages.length > 0" value="messages" header="Messages" class="h-full flex flex-column">
            <div class="messages-panel flex-grow-1 overflow-auto">
              <h4>Messages</h4>
              <pre class="p-2 border-1 surface-border border-round bg-gray-900 text-white">{{ response.messages.join('\n') }}</pre>
            </div>
          </TabPanel>
          
          <TabPanel v-if="response.results.length === 0 && response.messages.length === 0 && !isLoading && !errorMessage" value="status" header="Status">
            <div class="p-4 text-center">Query executed successfully. No rows returned and no messages.</div>
          </TabPanel>
        </TabView>
      </div>
      <div v-else-if="!isLoading && response && !errorMessage && response.results.length === 0 && response.messages.length === 0" class="results-panel p-4 text-center">
        <p>Query executed successfully. No rows returned and no messages.</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.query-view {
  display: flex;
  flex-direction: column;
}

.editor-container {
  height: 15rem; /* Fixed height for the editor */
  border: 1px solid var(--p-surface-border);
}

.results-area {
  flex-grow: 1;
  display: flex;
  flex-direction: column;
}

.results-header h4 {
  margin: 0;
}

.messages-panel pre {
  white-space: pre-wrap;
  word-wrap: break-word;
  font-family: monospace;
}

/* Ensure DataTable takes full height within its container */
:deep(.p-datatable-wrapper) {
  flex-grow: 1;
}

:deep(.p-datatable-table) {
  min-height: 100%;
}
</style>
