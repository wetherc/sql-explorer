<template>
  <v-tabs v-model="activeTabId" bg-color="primary" dark show-arrows>
    <v-tab v-for="tab in tabs" :key="tab.id" :value="tab.id">
      {{ tab.title }}
      <v-btn icon size="x-small" @click.stop="closeTab(tab.id)">
        <v-icon>mdi-close</v-icon>
      </v-btn>
    </v-tab>
    <v-btn icon @click="addTab">
      <v-icon>mdi-plus</v-icon>
    </v-btn>
  </v-tabs>

  <v-window v-model="activeTabId">
    <v-window-item v-for="tab in tabs" :key="tab.id" :value="tab.id" class="fill-height">
      <QueryView :initial-query="tab.query" :tab-id="tab.id" />
    </v-window-item>
  </v-window>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useTabsStore } from '@/stores/tabs'
import QueryView from './QueryView.vue'

const tabsStore = useTabsStore()

const tabs = computed(() => tabsStore.tabs)
const activeTabId = computed({
  get: () => tabsStore.activeTabId,
  set: (id) => {
    if (id) tabsStore.setActiveTab(id)
  },
})

const addTab = () => tabsStore.addTab()
const closeTab = (id: string) => tabsStore.closeTab(id)
</script>
