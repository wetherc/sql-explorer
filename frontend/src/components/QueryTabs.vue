<script setup lang="ts">
import { onMounted, watch, ref } from 'vue'
import { useTabsStore } from '@/stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { useQueryStore } from '@/stores/query'
import QueryView from '@/views/QueryView.vue'

import TabView from 'primevue/tabview'
import TabPanel from 'primevue/tabpanel'
import Button from 'primevue/button'

const tabsStore = useTabsStore()
const connectionStore = useConnectionStore()
const queryStore = useQueryStore()

const activeTabIndex = ref(0) // Local state for active tab index

// Add a default tab when the component mounts if none exist
onMounted(() => {
  if (tabsStore.tabs.value.length === 0) {
    tabsStore.addTab()
  }
})

// Watch for activeTabId changes to update activeTabIndex
watch(() => tabsStore.activeTabId.value, (newId) => { // .value added here
  const index = tabsStore.tabs.value.findIndex(tab => tab.id === newId)
  if (index !== -1) {
    activeTabIndex.value = index
  }
}, { immediate: true })

// Watch for activeTab changes to ensure queryStore is updated
watch(() => tabsStore.activeTab.value, (newTab) => {
  if (newTab) {
    queryStore.setQueryState(newTab.query, newTab.response, newTab.errorMessage, newTab.isLoading);
  }
}, { immediate: true });

async function handleExecuteQuery() {
  if (!tabsStore.activeTab.value) return;

  tabsStore.activeTab.value.isLoading = true;
  tabsStore.activeTab.value.errorMessage = null;
  tabsStore.activeTab.value.response = null; // Clear previous results

  try {
    const success = await queryStore.executeQuery(tabsStore.activeTab.value.query);
    tabsStore.activeTab.value.isLoading = false;

    if (success) {
      tabsStore.activeTab.value.response = queryStore.response;
      tabsStore.activeTab.value.errorMessage = null;
    } else {
      tabsStore.activeTab.value.errorMessage = queryStore.errorMessage;
      tabsStore.activeTab.value.response = null;
    }
  } catch (error: unknown) {
    tabsStore.activeTab.value.errorMessage = error instanceof Error ? error.message : 'An unknown error occurred.';
    tabsStore.activeTab.value.response = null;
    tabsStore.activeTab.value.isLoading = false;
  }
}

function updateQuery(newQuery: string) {
  if (tabsStore.activeTab.value) {
    tabsStore.activeTab.value.query = newQuery;
  }
}

function onTabClose(event: { index: number }) {
  const tab = tabsStore.tabs.value[event.index];
  if (tab) {
    const tabIdToClose = tab.id
    tabsStore.closeTab(tabIdToClose)
  }
}

</script>

<template>
  <div class="query-tabs-container">
    <header class="tab-header flex justify-content-between align-items-center p-2 surface-500">
      <TabView v-model:activeIndex="activeTabIndex" @tab-remove="onTabClose" :closable="true" class="flex-grow-1">
        <TabPanel v-for="tab in tabsStore.tabs.value" :key="tab.id" :value="tab.id" :header="tab.title" />
      </TabView>
      <div class="p-ml-2">
        <Button icon="pi pi-plus" class="p-button-rounded p-button-text p-button-sm" @click="tabsStore.addTab()" />
        <Button icon="pi pi-power-off" class="p-button-rounded p-button-text p-button-sm p-button-danger" @click="connectionStore.disconnect()" />
      </div>
    </header>

    <div class="tab-content flex-grow-1">
      <QueryView
        v-if="tabsStore.activeTab.value"
        :query="tabsStore.activeTab.value.query"
        :response="tabsStore.activeTab.value.response"
        :is-loading="tabsStore.activeTab.value.isLoading"
        :error-message="tabsStore.activeTab.value.errorMessage"
        @update:query="updateQuery"
        @execute-query="handleExecuteQuery"
      />
      <div v-else class="no-tab-selected p-4 text-center text-500">
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
  background-color: var(--p-content-background);
  color: var(--p-text-color);
  flex-shrink: 0;
  border-bottom: 1px solid var(--p-surface-border);
}

:deep(.p-tabview) {
  border: none;
  background: transparent;
}

:deep(.p-tabview-nav) {
  border: none;
  background: transparent;
}

:deep(.p-tabview-nav-link) {
  background: var(--p-surface-400);
  margin-right: 2px;
  border-top-left-radius: 4px;
  border-top-right-radius: 4px;
}

:deep(.p-tabview-nav-link.p-highlight) {
  background: var(--p-content-background);
  border-color: var(--p-surface-border);
  border-bottom-color: var(--p-content-background);
}

:deep(.p-tabview-nav-container) {
  border-bottom: 1px solid var(--p-surface-border);
}

:deep(.p-tabview-panels) {
  padding: 0;
}
</style>
