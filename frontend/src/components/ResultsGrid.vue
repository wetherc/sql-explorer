<template>
  <div class="results-grid">
    <div class="grid-toolbar d-flex align-center ga-1 px-2 py-1">
      <v-text-field
        v-model="search"
        density="compact"
        hide-details
        clearable
        placeholder="Filter rows"
        prepend-inner-icon="mdi-magnify"
        class="grid-filter"
        data-test="grid-filter"
      />
      <v-spacer />
      <span class="text-caption text-medium-emphasis mr-2" data-test="grid-count">
        {{ countLabel }}
      </span>
      <v-tooltip location="top" text="Copy the rows as text">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            icon="mdi-content-copy"
            size="small"
            aria-label="Copy the rows as text"
            data-test="grid-copy"
            @click="copyAll"
          />
        </template>
      </v-tooltip>
      <v-menu>
        <template #activator="{ props: menu }">
          <v-btn
            v-bind="menu"
            icon="mdi-download"
            size="small"
            aria-label="Export the rows"
            data-test="grid-export"
          />
        </template>
        <v-list density="compact">
          <v-list-item title="Export as CSV" @click="emit('export', 'csv')" />
          <v-list-item title="Export as JSON" @click="emit('export', 'json')" />
        </v-list>
      </v-menu>
    </div>

    <div v-if="result.truncated" class="px-3 py-1">
      <v-alert type="warning" density="compact" variant="tonal" data-test="grid-truncated">
        The row limit stopped the read at {{ result.rows.length.toLocaleString() }} rows.
      </v-alert>
    </div>

    <div ref="scroller" class="grid-scroll" @scroll="onScroll">
      <table class="grid-table">
        <thead>
          <tr>
            <th class="row-number">#</th>
            <th
              v-for="(column, index) in result.columns"
              :key="`${column.name}-${index}`"
              :class="{ sorted: sortIndex === index }"
              :aria-sort="ariaSort(index)"
              data-test="grid-header"
              @click="toggleSort(index)"
            >
              <span class="header-name">{{ column.name }}</span>
              <span class="header-type">{{ column.typeName }}</span>
              <v-icon v-if="sortIndex === index" size="x-small">
                {{ sortDescending ? 'mdi-arrow-down' : 'mdi-arrow-up' }}
              </v-icon>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="topPad > 0" :style="{ height: `${topPad}px` }">
            <td :colspan="result.columns.length + 1"></td>
          </tr>
          <tr v-for="entry in windowRows" :key="entry.index" data-test="grid-row">
            <td class="row-number">{{ entry.index + 1 }}</td>
            <td
              v-for="(cell, cellIndex) in entry.row"
              :key="cellIndex"
              :class="{ 'null-cell': isNullCell(cell) }"
              :title="formatCell(cell)"
              data-test="grid-cell"
              @click="inspect(cell, columnName(cellIndex))"
            >
              {{ truncate(formatCell(cell), 160) }}
            </td>
          </tr>
          <tr v-if="bottomPad > 0" :style="{ height: `${bottomPad}px` }">
            <td :colspan="result.columns.length + 1"></td>
          </tr>
          <tr v-if="sortedRows.length === 0">
            <td :colspan="result.columns.length + 1" class="text-center py-6 text-medium-emphasis">
              This statement returned no rows.
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <v-dialog v-model="inspecting" max-width="720">
      <v-card>
        <v-card-title class="text-subtitle-1">{{ inspectTitle }}</v-card-title>
        <v-card-text>
          <pre class="inspect-body">{{ inspectValue }}</pre>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Copy" @click="copyText(inspectValue)" />
          <v-btn text="Close" @click="inspecting = false" />
        </v-card-actions>
      </v-card>
    </v-dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { compareCells, formatCell, isNullCell, truncate } from '@/lib/format'
import { toTabSeparated } from '@/lib/export'
import type { CellValue, ResultSet } from '@/types/api'

const props = defineProps<{ result: ResultSet }>()
const emit = defineEmits<{
  (event: 'export', format: 'csv' | 'json'): void
  (event: 'copied', text: string): void
}>()

/** The height of one row, which the window of visible rows is built from. */
const ROW_HEIGHT = 30
/** The number of rows drawn above and below the visible area. */
const OVERSCAN = 12

const search = ref('')
const sortIndex = ref<number | null>(null)
const sortDescending = ref(false)
const scroller = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportHeight = ref(600)

const inspecting = ref(false)
const inspectValue = ref('')
const inspectTitle = ref('')

const filteredRows = computed(() => {
  const needle = search.value.trim().toLowerCase()
  if (needle === '') {
    return props.result.rows
  }
  return props.result.rows.filter((row) =>
    row.some((cell) => formatCell(cell).toLowerCase().includes(needle)),
  )
})

const sortedRows = computed(() => {
  const index = sortIndex.value
  if (index === null) {
    return filteredRows.value
  }
  const direction = sortDescending.value ? -1 : 1
  return [...filteredRows.value].sort(
    (left, right) => compareCells(left[index] ?? null, right[index] ?? null) * direction,
  )
})

const firstVisible = computed(() =>
  Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - OVERSCAN),
)
const visibleCount = computed(() => Math.ceil(viewportHeight.value / ROW_HEIGHT) + OVERSCAN * 2)
const lastVisible = computed(() =>
  Math.min(sortedRows.value.length, firstVisible.value + visibleCount.value),
)

const windowRows = computed(() =>
  sortedRows.value
    .slice(firstVisible.value, lastVisible.value)
    .map((row, offset) => ({ index: firstVisible.value + offset, row })),
)

const topPad = computed(() => firstVisible.value * ROW_HEIGHT)
const bottomPad = computed(() =>
  Math.max(0, (sortedRows.value.length - lastVisible.value) * ROW_HEIGHT),
)

const countLabel = computed(() => {
  const shown = sortedRows.value.length
  const total = props.result.rows.length
  return shown === total
    ? `${total.toLocaleString()} rows`
    : `${shown.toLocaleString()} of ${total.toLocaleString()} rows`
})

function onScroll(event: Event): void {
  const target = event.target as HTMLElement
  scrollTop.value = target.scrollTop
  viewportHeight.value = target.clientHeight || viewportHeight.value
}

function toggleSort(index: number): void {
  if (sortIndex.value === index) {
    if (sortDescending.value) {
      sortIndex.value = null
      sortDescending.value = false
    } else {
      sortDescending.value = true
    }
    return
  }
  sortIndex.value = index
  sortDescending.value = false
}

function ariaSort(index: number): 'ascending' | 'descending' | 'none' {
  if (sortIndex.value !== index) {
    return 'none'
  }
  return sortDescending.value ? 'descending' : 'ascending'
}

function columnName(index: number): string {
  return props.result.columns[index]?.name ?? ''
}

function inspect(cell: CellValue, name: string): void {
  inspectTitle.value = name
  inspectValue.value = formatCell(cell)
  inspecting.value = true
}

async function copyText(text: string): Promise<void> {
  const clipboard = globalThis.navigator?.clipboard
  if (clipboard) {
    await clipboard.writeText(text)
  }
  emit('copied', text)
}

function copyAll(): void {
  const header = props.result.columns.map((column) => column.name)
  void copyText(toTabSeparated([header, ...sortedRows.value]))
}

// A new result starts at the top with no sort and no filter.
watch(
  () => props.result,
  () => {
    search.value = ''
    sortIndex.value = null
    sortDescending.value = false
    scrollTop.value = 0
    if (scroller.value) {
      scroller.value.scrollTop = 0
    }
  },
)
</script>

<style scoped>
.results-grid {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.grid-filter {
  max-width: 260px;
}

.grid-scroll {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
}

.grid-table {
  border-collapse: separate;
  border-spacing: 0;
  width: max-content;
  min-width: 100%;
  font-size: 0.8125rem;
}

.grid-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: rgb(var(--v-theme-grid-header));
  text-align: left;
  padding: 4px 10px;
  cursor: pointer;
  white-space: nowrap;
  border-bottom: 1px solid rgb(var(--v-theme-surface-variant));
  user-select: none;
}

.grid-table th.sorted {
  color: rgb(var(--v-theme-primary));
}

.header-name {
  font-weight: 600;
}

.header-type {
  margin-left: 6px;
  font-weight: 400;
  font-size: 0.6875rem;
  opacity: 0.6;
}

.grid-table td {
  padding: 4px 10px;
  height: 30px;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  border-bottom: 1px solid rgba(var(--v-theme-surface-variant), 0.4);
  cursor: default;
}

.grid-table tbody tr:nth-child(even) td {
  background: rgb(var(--v-theme-grid-stripe));
}

.row-number {
  color: rgb(var(--v-theme-null-value));
  text-align: right;
  width: 1%;
  position: sticky;
  left: 0;
  background: rgb(var(--v-theme-surface));
}

.null-cell {
  color: rgb(var(--v-theme-null-value));
  font-style: italic;
}

.inspect-body {
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 50vh;
  overflow: auto;
  font-family: ui-monospace, monospace;
  font-size: 0.8125rem;
}
</style>
