<template>
  <div class="panel-header">
    <!-- A row for a control that changes what the panel shows, above the row
         that filters it. Most panels leave this row out. -->
    <div v-if="$slots.switch" class="panel-header-row px-2 pt-1">
      <slot name="switch" />
    </div>

    <div class="panel-header-row d-flex align-center ga-1 px-2 py-1">
      <v-text-field
        v-if="filter !== undefined"
        :model-value="filter"
        :placeholder="filterPlaceholder"
        :aria-label="filterLabel"
        :data-test="filterTestId"
        clearable
        prepend-inner-icon="mdi-magnify"
        class="panel-filter"
        @update:model-value="(value) => emit('update:filter', value ?? '')"
      />
      <slot name="lead" />
      <v-spacer />
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * The top row of one side panel or one result pane. The component holds the
 * padding, the gap and the lower border, so every panel starts with the same
 * shape.
 *
 * The filter field comes first in the row, in the same place in every panel.
 * A panel that does not filter its content leaves the `filter` property out.
 */
withDefaults(
  defineProps<{
    filter?: string
    filterPlaceholder?: string
    filterLabel?: string
    filterTestId?: string
  }>(),
  {
    filter: undefined,
    filterPlaceholder: 'Filter',
    filterLabel: undefined,
    filterTestId: undefined,
  },
)

const emit = defineEmits<{ 'update:filter': [string] }>()
</script>

<style scoped>
.panel-header {
  flex: 0 0 auto;
  border-bottom: var(--app-divider);
}

.panel-filter {
  max-width: 260px;
}
</style>
