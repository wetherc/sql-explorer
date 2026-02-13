<template>
  <v-tabs v-model="activeTabId" show-arrows>
    <v-tab v-for="tab in tabs" :key="tab.id" :value="tab.id" class="query-tab">
      {{ tab.title }}
      <v-icon size="x-small" @click.stop="closeTab(tab.id)">mdi-close</v-icon>
    </v-tab>
    <v-tab @click="addTab" :disabled="!selectedExplorerConnectionId">
      <v-icon start>mdi-plus</v-icon>
      New Query
    </v-tab>
  </v-tabs>

  <v-window v-model="activeTabId">
    <v-window-item v-for="tab in tabs" :key="tab.id" :value="tab.id" class="fill-height">
      <QueryView :initial-query="tab.query" :tab-id="tab.id" />
    </v-window-item>
  </v-window>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { useTabsStore } from '@/stores/tabs'
import { useNavigationStore } from '@/stores/navigation'
import QueryView from './QueryView.vue'

const tabsStore = useTabsStore()
const navigationStore = useNavigationStore()

const { selectedExplorerConnectionId } = storeToRefs(navigationStore)

const tabs = computed(() => tabsStore.tabs)
const activeTabId = computed({
  get: () => tabsStore.activeTabId,
  set: (id) => {
    if (id) tabsStore.setActiveTab(id)
  },
})

const addTab = () => {
  if (selectedExplorerConnectionId.value) {
    tabsStore.addTab(selectedExplorerConnectionId.value)
  }
}
const closeTab = (id: string) => tabsStore.closeTab(id)
</script>

<style scoped>
.v-tabs {
  background-color: #272727; /* Dark background for the tab bar area */
}

.query-tab {
  background-color: #333333; /* Lighter background for inactive tabs */
  border-top-left-radius: 6px;
  border-top-right-radius: 6px;
  margin-right: 2px;
}

.query-tab.v-tab--selected {
  background-color: #1E1E1E; /* Match a common editor background color */
  color: white;
}

/* Style the close icon on the active tab to be visible */
.query-tab.v-tab--selected .v-icon {
  color: white;
}
</style>
