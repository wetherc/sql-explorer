<script setup lang="ts">
import { onMounted, watch } from 'vue'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { useQueryStore } from '@/stores/query'
import QueryView from '@/views/QueryView.vue'
import { type QueryResponse } from '@/types/query'

const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()
const queryStore = useQueryStore()

// Add a default tab when the component mounts if none exist
onMounted(() => {
  if (tabsStore.tabs.length === 0) {
    tabsStore.addTab()
  }
})

// Watch for activeTab changes to ensure queryStore is updated
watch(() => tabsStore.activeTab, (newTab) => {
  if (newTab) {
    queryStore.setQueryState(newTab.query, newTab.response, newTab.errorMessage, newTab.isLoading);
  }
}, { immediate: true });

async function handleExecuteQuery() {
  if (!tabsStore.activeTab) return;

  tabsStore.activeTab.isLoading = true;
  tabsStore.activeTab.errorMessage = null;
  tabsStore.activeTab.response = null; // Clear previous results

  try {
    const success = await queryStore.executeQuery(tabsStore.activeTab.query);
    tabsStore.activeTab.isLoading = false;

    if (success) {
      tabsStore.activeTab.response = queryStore.response;
      tabsStore.activeTab.errorMessage = null;
    } else {
      tabsStore.activeTab.errorMessage = queryStore.errorMessage;
      tabsStore.activeTab.response = null;
    }
  } catch (error: any) {
    tabsStore.activeTab.errorMessage = error.message || 'An unknown error occurred.';
    tabsStore.activeTab.response = null;
    tabsStore.activeTab.isLoading = false;
  }
}

function updateQuery(newQuery: string) {
  if (tabsStore.activeTab) {
    tabsStore.activeTab.query = newQuery;
  }
}
</script>

<template>
  <div class="query-tabs-container">
    <header class="tab-header">
      <div class="tabs">
        <div
          v-for="tab in tabsStore.tabs"
          :key="tab.id"
          :class="['tab-item', { active: tab.id === tabsStore.activeTabId }]"
          @click="tabsStore.setActiveTab(tab.id)"
        >
          <span>{{ tab.title }}</span>
          <button class="close-tab" @click.stop="tabsStore.closeTab(tab.id)">x</button>
        </div>
        <button class="add-tab" @click="tabsStore.addTab()">+</button>
      </div>
      <button @click="connectionStore.disconnect()">Disconnect</button>
    </header>

    <div class="tab-content">
      <QueryView
        v-if="tabsStore.activeTab"
        :query="tabsStore.activeTab.query"
        :response="tabsStore.activeTab.response"
        :is-loading="tabsStore.activeTab.isLoading"
        :error-message="tabsStore.activeTab.errorMessage"
        @update:query="updateQuery"
        @execute-query="handleExecuteQuery"
      />
      <div v-else class="no-tab-selected">
        Please open a new query tab.
      </div>
    </div>
  </div>
</template>

<style scoped>
.query-tabs-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.tab-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  background-color: #343a40;
  color: white;
  padding: 0.5rem 1rem;
  flex-shrink: 0;
  border-bottom: 1px solid #212529;
}

.tabs {
  display: flex;
  overflow-x: auto;
  flex-grow: 1;
}

.tab-item {
  display: flex;
  align-items: center;
  padding: 0.5rem 1rem;
  cursor: pointer;
  border: 1px solid transparent;
  border-bottom: none;
  background-color: #495057;
  margin-right: 2px;
  border-radius: 5px 5px 0 0;
}

.tab-item.active {
  background-color: #f8f9fa;
  color: #212529;
  border-color: #dee2e6;
  border-bottom-color: #f8f9fa; /* Hide bottom border */
}

.close-tab {
  margin-left: 10px;
  background: none;
  border: none;
  color: white;
  font-weight: bold;
  cursor: pointer;
  padding: 0;
  line-height: 1;
}

.tab-item.active .close-tab {
  color: #212529;
}

.close-tab:hover {
  color: #dc3545;
}

.add-tab {
  background-color: #007bff;
  color: white;
  border: none;
  border-radius: 5px;
  padding: 0.5rem 1rem;
  margin-left: 5px;
  cursor: pointer;
}

.tab-content {
  flex-grow: 1;
  overflow: hidden;
  background-color: #f8f9fa;
}

.no-tab-selected {
  padding: 20px;
  text-align: center;
  color: #6c757d;
}

button {
  padding: 0.5rem 1rem;
  font-size: 1rem;
  color: #fff;
  background-color: #007bff;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}

button:disabled {
  background-color: #5a9ed8;
  cursor: not-allowed;
}
</style>
