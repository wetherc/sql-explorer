<template>
  <!-- The height of one row reaches the stylesheet as a custom property, so
       the window of visible rows and the cells that draw it hold one figure
       between them. -->
  <div class="results-grid" :style="{ '--grid-row-height': `${ROW_HEIGHT}px` }">
    <PanelHeader
      v-model:filter="search"
      filter-placeholder="Filter rows"
      filter-label="Filter the rows"
      filter-test-id="grid-filter"
    >
      <template #actions>
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
            <v-list-item
              v-for="entry in exportItems"
              :key="entry.format"
              :title="entry.title"
              data-test="grid-export-item"
              @click="askExport(entry.format)"
            />
            <template v-if="result.truncated">
              <v-divider />
              <v-list-item
                title="Write every row to a CSV file"
                subtitle="Runs the statement again and writes from the server"
                data-test="grid-export-all-csv"
                @click="emit('export-all', 'csv')"
              />
              <v-list-item
                title="Write every row to a JSON file"
                subtitle="Runs the statement again and writes from the server"
                data-test="grid-export-all-json"
                @click="emit('export-all', 'json')"
              />
            </template>
            <v-divider v-if="hasSelection" />
            <v-list-item
              v-if="hasSelection"
              title="Clear the selection"
              data-test="grid-clear-selection"
              @click="clearSelection()"
            />
          </v-list>
        </v-menu>
      </template>
    </PanelHeader>

    <div v-if="result.truncated" class="px-3 py-1">
      <v-alert type="warning" density="compact" variant="tonal" data-test="grid-truncated">
        The row limit stopped the read at {{ result.rows.length.toLocaleString() }} rows.
      </v-alert>
    </div>

    <div ref="scrollArea" :key="resultGeneration" class="grid-scroll" @scroll="onScroll">
      <table class="grid-table">
        <thead>
          <tr>
            <th class="row-number">#</th>
            <th
              v-for="(column, index) in result.columns"
              :key="`${column.name}-${index}`"
              :class="{ sorted: sortIndex === index }"
              :aria-sort="ariaSort(index)"
              data-test="grid-header-cell"
            >
              <!-- The button carries the sort, so the key and the pointer
                   reach it the same way. -->
              <button
                type="button"
                class="header-button"
                data-test="grid-header"
                @click="toggleSort(index)"
              >
                <span class="header-name">{{ column.name }}</span>
                <span class="header-type">{{ column.typeName }}</span>
                <v-icon v-if="sortIndex === index" size="x-small" aria-hidden="true">
                  {{ sortDescending ? 'mdi-arrow-down' : 'mdi-arrow-up' }}
                </v-icon>
              </button>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="topPad > 0" :style="{ height: `${topPad}px` }">
            <td :colspan="result.columns.length + 1"></td>
          </tr>
          <tr
            v-for="entry in windowRows"
            :key="entry.sourceIndex"
            :class="{ selected: selected.has(entry.sourceIndex) }"
            :aria-selected="selected.has(entry.sourceIndex)"
            data-test="grid-row"
            @click="onRowClick(entry.position, entry.sourceIndex, $event)"
          >
            <td class="row-number">{{ entry.position + 1 }}</td>
            <td
              v-for="(cell, cellIndex) in entry.row"
              :key="cellIndex"
              :class="{ 'null-cell': isNullCell(cell) }"
              :title="entry.texts[cellIndex]"
              data-test="grid-cell"
              @dblclick="inspect(cell, columnName(cellIndex))"
            >
              {{ truncate(entry.texts[cellIndex] ?? '', 160) }}
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
          <pre class="app-code-block">{{ inspectValue }}</pre>
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
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import PanelHeader from './PanelHeader.vue'
import { compareCells, formatCell, isNullCell, truncate } from '@/lib/format'
import { toTabSeparated } from '@/lib/export'
import type { CellValue, ResultSet } from '@/types/api'

/** The forms an export can take. */
export type ExportFormat = 'csv' | 'json' | 'markdown' | 'insert' | 'xlsx'

const props = defineProps<{ result: ResultSet }>()
const emit = defineEmits<{
  (event: 'export', format: ExportFormat, rows: ResultSet): void
  (event: 'export-all', format: 'csv' | 'json'): void
  (event: 'copied', text: string): void
}>()

/** The height of one row, which the window of visible rows is built from. */
const ROW_HEIGHT = 30
/** The number of rows drawn above and below the visible area. */
const OVERSCAN = 12

const search = ref('')
/**
 * The filter text the rows are matched against. It follows the field after
 * a short pause, so that a keystroke does not scan every row at once.
 */
const appliedSearch = ref('')
const FILTER_DELAY_MS = 200
let filterTimer: ReturnType<typeof setTimeout> | null = null
watch(search, (value) => {
  if (filterTimer !== null) {
    clearTimeout(filterTimer)
  }
  filterTimer = setTimeout(() => {
    filterTimer = null
    appliedSearch.value = (value ?? '').trim().toLowerCase()
  }, FILTER_DELAY_MS)
})

const sortIndex = ref<number | null>(null)
const sortDescending = ref(false)
/** Rises with each new result, which draws a fresh scroll area. */
const resultGeneration = ref(0)
const scrollTop = ref(0)
const viewportHeight = ref(600)
/** The element the rows scroll in, which gives the height of the window. */
const scrollArea = ref<HTMLElement | null>(null)
let sizeObserver: ResizeObserver | null = null

onMounted(() => {
  if (typeof ResizeObserver === 'undefined') {
    return
  }
  sizeObserver = new ResizeObserver(() => {
    const element = scrollArea.value
    if (element && element.clientHeight > 0) {
      viewportHeight.value = element.clientHeight
    }
  })
  if (scrollArea.value) {
    sizeObserver.observe(scrollArea.value)
  }
})

// A new result draws a fresh scroll area, so the observer moves with it.
watch(scrollArea, (element, previous) => {
  if (previous) {
    sizeObserver?.unobserve(previous)
  }
  if (element) {
    sizeObserver?.observe(element)
  }
})

onBeforeUnmount(() => {
  sizeObserver?.disconnect()
  if (filterTimer !== null) {
    clearTimeout(filterTimer)
  }
})

const inspecting = ref(false)
const inspectValue = ref('')
const inspectTitle = ref('')

/**
 * The rows the user selected, held by their place in the result and not by
 * their place in the view. A sort or a filter moves a row in the view, and
 * the selection must follow the row and not the position.
 */
const selected = ref(new Set<number>())
/** The row of the last plain click, from which a click with Shift reaches. */
const anchor = ref<number | null>(null)

/** Every row of the result with the place it holds in the result. */
const sourceRows = computed(() =>
  props.result.rows.map((row, sourceIndex) => ({ row, sourceIndex })),
)

/**
 * The text of every row in small letters, which the filter matches against.
 * A computed property runs only when it is read, so this text is built the
 * first time a filter is active and once for each result after that.
 */
const rowTexts = computed(() =>
  props.result.rows.map((row) =>
    row
      .map((cell) => formatCell(cell))
      .join(' ')
      .toLowerCase(),
  ),
)

const filteredRows = computed(() => {
  const needle = appliedSearch.value
  if (needle === '') {
    return sourceRows.value
  }
  return sourceRows.value.filter((entry) => rowTexts.value[entry.sourceIndex]?.includes(needle))
})

const sortedRows = computed(() => {
  const index = sortIndex.value
  if (index === null) {
    return filteredRows.value
  }
  const direction = sortDescending.value ? -1 : 1
  return [...filteredRows.value].sort(
    (left, right) => compareCells(left.row[index] ?? null, right.row[index] ?? null) * direction,
  )
})

const hasSelection = computed(() => selected.value.size > 0)

const exportItems = computed<Array<{ format: ExportFormat; title: string }>>(() => {
  const scope = hasSelection.value ? 'the selected rows' : 'the rows'
  return [
    { format: 'csv', title: `Export ${scope} as CSV` },
    { format: 'json', title: `Export ${scope} as JSON` },
    { format: 'markdown', title: `Export ${scope} as Markdown` },
    { format: 'insert', title: `Export ${scope} as INSERT statements` },
    { format: 'xlsx', title: `Export ${scope} as Excel` },
  ]
})

const firstVisible = computed(() =>
  Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - OVERSCAN),
)
const visibleCount = computed(() => Math.ceil(viewportHeight.value / ROW_HEIGHT) + OVERSCAN * 2)
const lastVisible = computed(() =>
  Math.min(sortedRows.value.length, firstVisible.value + visibleCount.value),
)

const windowRows = computed(() =>
  sortedRows.value.slice(firstVisible.value, lastVisible.value).map((entry, offset) => ({
    ...entry,
    position: firstVisible.value + offset,
    // The text of each cell is built once here, so the view does not build
    // it again for the tooltip and for the body of the cell.
    texts: entry.row.map((cell) => formatCell(cell)),
  })),
)

const topPad = computed(() => firstVisible.value * ROW_HEIGHT)
const bottomPad = computed(() =>
  Math.max(0, (sortedRows.value.length - lastVisible.value) * ROW_HEIGHT),
)

const countLabel = computed(() => {
  const shown = sortedRows.value.length
  const total = props.result.rows.length
  const head =
    shown === total
      ? `${total.toLocaleString()} rows`
      : `${shown.toLocaleString()} of ${total.toLocaleString()} rows`
  return hasSelection.value ? `${head}, ${selected.value.size.toLocaleString()} selected` : head
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

/**
 * Answers a click on a row. A plain click takes the row alone. A click with
 * Control or Command adds the row or takes it away. A click with Shift takes
 * every row between the last plain click and this one.
 */
function onRowClick(position: number, sourceIndex: number, event: MouseEvent): void {
  if (event.shiftKey && anchor.value !== null) {
    const from = Math.min(anchor.value, position)
    const to = Math.max(anchor.value, position)
    const next = new Set(selected.value)
    for (const entry of sortedRows.value.slice(from, to + 1)) {
      next.add(entry.sourceIndex)
    }
    selected.value = next
    return
  }
  if (event.ctrlKey || event.metaKey) {
    const next = new Set(selected.value)
    if (next.has(sourceIndex)) {
      next.delete(sourceIndex)
    } else {
      next.add(sourceIndex)
    }
    selected.value = next
    anchor.value = position
    return
  }
  selected.value = new Set([sourceIndex])
  anchor.value = position
}
function clearSelection(): void {
  selected.value = new Set()
  anchor.value = null
}

/**
 * Builds the result an export writes. The rows follow the sort and the
 * filter of the view, and they hold the selection alone when there is one.
 */
function rowsToExport(): ResultSet {
  const entries = hasSelection.value
    ? sortedRows.value.filter((entry) => selected.value.has(entry.sourceIndex))
    : sortedRows.value
  return {
    columns: props.result.columns,
    rows: entries.map((entry) => entry.row),
    truncated: props.result.truncated,
  }
}

function askExport(format: ExportFormat): void {
  emit('export', format, rowsToExport())
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
  void copyText(toTabSeparated([header, ...rowsToExport().rows]))
}

// A sort or a filter moves the rows in the view, so the anchor of a click
// with Shift no longer points at the row the user last clicked.
watch([sortIndex, sortDescending, appliedSearch], () => {
  anchor.value = null
})

// A new result starts at the top with no sort and no filter.
watch(
  () => props.result,
  () => {
    if (filterTimer !== null) {
      clearTimeout(filterTimer)
      filterTimer = null
    }
    search.value = ''
    appliedSearch.value = ''
    sortIndex.value = null
    sortDescending.value = false
    scrollTop.value = 0
    resultGeneration.value += 1
    clearSelection()
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
  font-size: var(--app-text-md);
}

.grid-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: rgb(var(--v-theme-grid-header));
  text-align: left;
  padding: 0;
  white-space: nowrap;
  border-bottom: var(--app-divider);
}

.header-button {
  display: block;
  width: 100%;
  padding: 4px 10px;
  background: none;
  border: 0;
  color: inherit;
  font: inherit;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
}

.header-button:hover {
  background: rgba(var(--v-theme-on-surface), 0.08);
}

.header-button:focus-visible {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
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
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-on-grid-header));
}

.grid-table td {
  padding: 4px 10px;
  height: var(--grid-row-height);
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  border-bottom: var(--app-divider-soft);
  cursor: default;
}

/* The three rules below give a cell its background, and they carry the same
   weight as each other. The order is therefore what decides: a stripe covers
   the plain cell, the gutter of row numbers covers the stripe, and the mark of
   a chosen row covers both. */
.grid-table tbody tr:nth-child(even) td {
  background: rgb(var(--v-theme-grid-stripe));
}

/* The column of row numbers holds still while the rows scroll sideways, so it
   needs a background of its own for the cells to pass behind. It takes the
   colour of the column headers, because it names a row as they name a column. */
.grid-table tbody tr td.row-number {
  background: rgb(var(--v-theme-grid-header));
}

.grid-table tbody tr.selected td {
  background: rgba(var(--v-theme-primary), 0.16);
}

.row-number {
  color: rgb(var(--v-theme-on-grid-header));
  text-align: right;
  width: 1%;
  position: sticky;
  left: 0;
}

/* The corner cell holds still in both directions, so it stands above the
   other header cells, which pass behind it as the rows scroll sideways. */
.grid-table th.row-number {
  z-index: 2;
}

.null-cell {
  color: rgb(var(--v-theme-null-value));
  font-style: italic;
}
</style>
