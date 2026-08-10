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
        <!-- A tab is itself a button, so the close mark inside it cannot be a
             second button. The mark answers the mouse, and the Delete key on
             the tab closes it for a user of the keyboard. -->
        <v-tab
          v-for="tab in tabs.tabs"
          :key="tab.id"
          :value="tab.id"
          class="query-tab"
          data-test="query-tab"
          @keydown.delete.prevent="tabs.close(tab.id)"
        >
          <span class="tab-title">{{ tab.title }}</span>
          <span v-if="tab.dirty" class="dirty-mark" aria-hidden="true">●</span>
          <span v-if="tab.dirty" class="app-visually-hidden">, has changes</span>
          <v-icon
            size="x-small"
            class="ml-2 close-mark"
            aria-hidden="true"
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
      <!-- A tab mounts its view, with its editor, the first time the user
           opens it. A workspace with many tabs then starts with one editor
           and not one for each tab. -->
      <template v-for="tab in tabs.tabs" :key="tab.id">
        <QueryView
          v-if="visitedTabIds.has(tab.id)"
          v-show="tab.id === tabs.activeTabId"
          :tab="tab"
        />
      </template>

      <EmptyState
        v-if="!tabs.hasTabs"
        size="page"
        icon="mdi-database-search-outline"
        title="No open tabs"
        :hint="emptyHint"
      >
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
      </EmptyState>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import EmptyState from './EmptyState.vue'
import QueryView from './QueryView.vue'
import { useConnectionsStore } from '@/stores/connections'
import { useTabsStore } from '@/stores/tabs'

const tabs = useTabsStore()
const connections = useConnectionsStore()

/** The tabs the user has opened at least once in this session. */
const visitedTabIds = ref(new Set<string>())
watch(
  () => tabs.activeTabId,
  (id) => {
    if (id !== null && !visitedTabIds.value.has(id)) {
      visitedTabIds.value = new Set(visitedTabIds.value).add(id)
    }
  },
  { immediate: true },
)

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
  border-bottom: var(--app-divider);
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
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-warning));
}

/* The mark grows a background under the pointer, so the area it answers is
   plain to see before the click. */
.close-mark {
  border-radius: 50%;
  padding: 2px;
}

.close-mark:hover {
  background: rgba(var(--v-theme-on-surface), 0.12);
}

.tab-body {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
}

.tab-body > * {
  height: 100%;
}
</style>
