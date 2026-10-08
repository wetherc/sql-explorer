<template>
  <v-menu v-if="limits.length > 0" :close-on-content-click="false" location="bottom start">
    <template #activator="{ props: menu }">
      <v-btn
        v-bind="menu"
        variant="text"
        size="small"
        color="warning"
        prepend-icon="mdi-alert-circle-outline"
        text="Autocomplete is limited"
        aria-label="Autocomplete is limited. Show the databases it can't fully read."
        data-test="autocomplete-status"
      />
    </template>
    <v-card max-width="480" data-test="autocomplete-limits">
      <v-card-text>
        <p class="mb-3">Autocomplete is missing names from these databases.</p>
        <section
          v-for="limit in limits"
          :key="limit.database"
          class="limit"
          :aria-label="limit.database"
          data-test="autocomplete-limit"
        >
          <p data-test="autocomplete-limit-message">{{ limit.message }}</p>
          <pre
            v-if="limit.detail"
            class="app-code-block limit-detail"
            data-test="autocomplete-limit-detail"
            >{{ limit.detail }}</pre>
          <v-btn
            size="small"
            variant="tonal"
            prepend-icon="mdi-refresh"
            text="Try again"
            :loading="retrying.has(limit.database)"
            :aria-label="`Read the schema of ${limit.database} again`"
            data-test="autocomplete-retry"
            @click="retry(connectionId as string, limit.database)"
          />
        </section>
      </v-card-text>
    </v-card>
  </v-menu>
</template>

<script setup lang="ts">
/**
 * A quiet marker beside the connection of a query tab. It shows only when
 * the editor offers the names of a database of that connection in part or
 * not at all. A click opens a menu that gives the cause for each database,
 * with a button that reads the schema again at once.
 */
import { computed, reactive } from 'vue'
import { useExplorerStore } from '@/stores/explorer'

const props = defineProps<{ connectionId: string | null }>()

const explorer = useExplorerStore()

/** The databases whose read runs because the user asked to try again. */
const retrying = reactive(new Set<string>())

const limits = computed(() => explorer.autocompleteLimits(props.connectionId))

async function retry(connectionId: string, database: string): Promise<void> {
  retrying.add(database)
  await explorer.retrySnapshot(connectionId, database)
  retrying.delete(database)
}
</script>

<style scoped>
.limit + .limit {
  margin-top: 12px;
  padding-top: 12px;
  border-top: var(--app-divider);
}

.limit-detail {
  margin: 6px 0 8px;
  max-height: 160px;
  overflow: auto;
  white-space: pre-wrap;
}
</style>
