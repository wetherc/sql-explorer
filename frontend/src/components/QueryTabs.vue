<template>
  <div class="query-tabs">
    <div class="tab-strip d-flex align-center">
      <v-tabs
        :model-value="tabs.activeTabId"
        density="compact"
        show-arrows
        class="flex-grow-1"
        @update:model-value="(id) => tabs.activate(String(id))"
      >
        <v-tab
          v-for="tab in tabs.tabs"
          :key="tab.id"
          :value="tab.id"
          class="query-tab"
          data-test="query-tab"
        >
          <span class="tab-title">{{ tab.title }}</span>
          <span v-if="tab.dirty" class="dirty-mark" aria-label="This tab has changes">●</span>
          <v-icon
            size="x-small"
            class="ml-2"
            aria-label="Close the tab"
            data-test="close-tab"
            @click.stop="tabs.close(tab.id)"
          >
            mdi-close
          </v-icon>
        </v-tab>
      </v-tabs>
      <v-tooltip location="bottom" text="Open a new tab">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            icon="mdi-plus"
            size="small"
            aria-label="Open a new tab"
            data-test="new-tab"
            @click="newTab"
          />
        </template>
      </v-tooltip>
    </div>

    <div class="tab-body">
      <template v-for="tab in tabs.tabs" :key="tab.id">
        <QueryView v-show="tab.id === tabs.activeTabId" :tab="tab" />
      </template>

      <div v-if="!tabs.hasTabs" class="empty-state">
        <v-icon size="56" color="primary" class="mb-3">mdi-database-search-outline</v-icon>
        <div class="text-h6 mb-1">No open tabs</div>
        <p class="text-body-2 text-medium-emphasis mb-4">
          {{ emptyHint }}
        </p>
        <v-btn
          v-if="connections.hasActive"
          color="primary"
          variant="flat"
          prepend-icon="mdi-plus"
          text="New query"
          data-test="empty-new-tab"
          @click="newTab"
        />
        <v-btn
          v-else
          color="primary"
          variant="flat"
          prepend-icon="mdi-lan-connect"
          text="Open the connections"
          data-test="empty-open-connections"
          @click="emit('open-connections')"
        />
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import QueryView from './QueryView.vue'
import { useConnectionsStore } from '@/stores/connections'
import { useTabsStore } from '@/stores/tabs'

const tabs = useTabsStore()
const connections = useConnectionsStore()

const emit = defineEmits<{ (event: 'open-connections'): void }>()

const emptyHint = computed(() =>
  connections.hasActive
    ? 'Open a tab to write a statement against the connection you selected.'
    : 'Open a connection first. Its objects then appear in the explorer.',
)

function newTab(): void {
  tabs.add()
}
</script>

<style scoped>
.query-tabs {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.tab-strip {
  flex: 0 0 auto;
  background: rgb(var(--v-theme-surface-light));
  border-bottom: 1px solid rgb(var(--v-theme-surface-variant));
}

.query-tab {
  text-transform: none;
  letter-spacing: normal;
}

.tab-title {
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dirty-mark {
  margin-left: 6px;
  font-size: 0.7rem;
  color: rgb(var(--v-theme-warning));
}

.tab-body {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
}

.tab-body > * {
  height: 100%;
}

.empty-state {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 24px;
}
</style>
