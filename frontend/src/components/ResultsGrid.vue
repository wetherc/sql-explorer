<template>
  <!-- The height of one row reaches the stylesheet as a custom property, so
       the window of visible rows and the cells that draw it hold one figure
       between them. -->
  <div class="results-grid" :style="{ '--grid-row-height': `${ROW_HEIGHT}px` }">
    <PanelHeader
      v-model:filter="search"
      filter-placeholder="Filter rows"
      filter-label="Filter rows"
      filter-test-id="grid-filter"
    >
      <template #actions>
        <span
          v-if="filterProgress !== null"
          class="text-caption text-medium-emphasis mr-2"
          role="status"
          data-test="grid-filtering"
        >
          Filtering… {{ Math.round(filterProgress * 100) }}%
        </span>
        <span class="text-caption text-medium-emphasis mr-2" data-test="grid-count">
          {{ countLabel }}
        </span>
        <v-menu>
          <template #activator="{ props: menu }">
            <v-btn
              v-bind="menu"
              icon="mdi-content-copy"
              size="small"
              aria-label="Copy rows"
              data-test="grid-copy"
            />
          </template>
          <v-list density="compact">
            <v-list-item
              title="Copy with headers"
              data-test="grid-copy-with-names"
              @click="copyAll"
            />
            <v-list-item
              title="Copy without headers"
              data-test="grid-copy-without-names"
              @click="copyRowsOnly"
            />
          </v-list>
        </v-menu>
        <v-menu>
          <template #activator="{ props: menu }">
            <v-btn
              v-bind="menu"
              icon="mdi-download"
              size="small"
              aria-label="Export rows"
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
            <template v-if="truncated">
              <v-divider />
              <v-list-item
                title="Export all rows to CSV"
                subtitle="Re-runs the query and streams rows from the server"
                data-test="grid-export-all-csv"
                @click="emit('export-all', 'csv')"
              />
              <v-list-item
                title="Export all rows to JSON"
                subtitle="Re-runs the query and streams rows from the server"
                data-test="grid-export-all-json"
                @click="emit('export-all', 'json')"
              />
              <v-list-item
                title="Export all rows to Excel"
                subtitle="Re-runs the query and streams rows from the server"
                data-test="grid-export-all-xlsx"
                @click="emit('export-all', 'xlsx')"
              />
            </template>
            <v-divider v-if="hasSelection" />
            <v-list-item
              v-if="hasSelection"
              title="Clear selection"
              data-test="grid-clear-selection"
              @click="clearSelection()"
            />
          </v-list>
        </v-menu>
      </template>
    </PanelHeader>

    <div v-if="truncated" class="px-3 py-1">
      <v-alert type="warning" density="compact" variant="tonal" data-test="grid-truncated">
        Showing the first {{ rowTotal.toLocaleString() }} rows because of the row limit.
      </v-alert>
    </div>

    <!-- While a new statement runs, the rows on screen belong to the one
         before it. The cover says so and holds off a click that would act on
         rows the user is about to lose. -->
    <div class="grid-body">
      <div v-if="busy" class="grid-busy" data-test="grid-busy">
        <v-progress-circular indeterminate size="20" width="2" />
        <span class="text-caption">Running…</span>
      </div>
      <div ref="scrollArea" :key="resultGeneration" class="grid-scroll" @scroll="onScroll">
        <!-- The grid draws only the rows around the visible part, so it reports
           the whole count and the place of each row. A reader would otherwise
           hear the thirty rows that are drawn as the whole result. -->
        <table
          class="grid-table"
          role="grid"
          aria-label="Result rows"
          :aria-rowcount="sortedOrder.length + 1"
          :aria-colcount="result.columns.length + 1"
          :aria-busy="busy"
          @keydown="onGridKeyDown"
          @copy="onGridCopy"
        >
          <thead>
            <tr ref="headerRow" role="row" aria-rowindex="1">
              <th class="row-number" role="columnheader" aria-colindex="1" scope="col">#</th>
              <th
                v-for="(column, index) in result.columns"
                :key="`${column.name}-${index}`"
                :class="{ sorted: sortIndex === index }"
                :style="headerStyle(index)"
                role="columnheader"
                scope="col"
                :aria-colindex="index + 2"
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
                <!-- The grip changes the width of the column. It is a control
                     of its own, so the arrow keys reach it as well. -->
                <span
                  class="column-grip"
                  role="separator"
                  aria-orientation="vertical"
                  :aria-label="`Resize column ${column.name}`"
                  tabindex="0"
                  data-test="grid-column-grip"
                  @pointerdown="startResize($event, index)"
                  @keydown="onGripKeyDown($event, index)"
                  @dblclick="clearWidth(index)"
                ></span>
              </th>
            </tr>
          </thead>
          <tbody>
            <!-- The two rows that hold the empty space above and below the drawn
               rows stand outside the reading, because they name nothing. -->
            <tr v-if="topPad > 0" :style="{ height: `${topPad}px` }" aria-hidden="true">
              <td :colspan="result.columns.length + 1"></td>
            </tr>
            <tr
              v-for="entry in windowRows"
              :key="entry.sourceIndex"
              :class="{ selected: selected.has(entry.sourceIndex) }"
              role="row"
              :aria-rowindex="entry.position + 2"
              :aria-selected="selected.has(entry.sourceIndex)"
              data-test="grid-row"
              @click="onRowClick(entry.position, entry.sourceIndex, $event)"
            >
              <td class="row-number" role="rowheader" aria-colindex="1">
                {{ entry.position + 1 }}
              </td>
              <td
                v-for="(cell, cellIndex) in entry.row"
                :key="cellIndex"
                :ref="(element) => keepCell(entry.position, cellIndex, element)"
                :class="{
                  'null-cell': isNullCell(cell),
                  'focused-cell': isFocused(entry, cellIndex),
                }"
                role="gridcell"
                :aria-colindex="cellIndex + 2"
                :tabindex="isFocused(entry, cellIndex) ? 0 : -1"
                data-test="grid-cell"
                @focus="onCellFocus($event, entry.position, cellIndex)"
                @mouseenter="revealFullValue($event, entry.position, cellIndex)"
                @dblclick="inspect(cell, columnName(cellIndex))"
                @contextmenu.prevent="openCellMenu($event, entry.position, cellIndex)"
              >
                <!-- The width of a cell is capped on this element and not on
                     the cell itself, because a table of automatic width pays
                     no attention to a cap on one of its cells. -->
                <span class="cell-text" :style="cellStyle(cellIndex)">
                  {{ truncate(entry.texts[cellIndex] ?? '', CELL_LIMIT) }}
                </span>
              </td>
            </tr>
            <tr v-if="bottomPad > 0" :style="{ height: `${bottomPad}px` }" aria-hidden="true">
              <td :colspan="result.columns.length + 1"></td>
            </tr>
            <tr v-if="sortedOrder.length === 0">
              <td
                :colspan="result.columns.length + 1"
                class="text-center py-6 text-medium-emphasis"
                data-test="grid-empty"
              >
                {{ emptyMessage }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <v-menu v-model="cellMenu.open" :target="[cellMenu.x, cellMenu.y]" data-test="grid-cell-menu">
      <v-list density="compact" min-width="240">
        <v-list-item title="Copy cell" data-test="grid-menu-copy-cell" @click="copyCellOfMenu" />
        <v-list-item title="Copy row" data-test="grid-menu-copy-row" @click="copyRowOfMenu" />
        <v-list-item
          title="Copy with headers"
          data-test="grid-menu-copy-with-names"
          @click="copyAll"
        />
        <v-list-item
          title="Copy without headers"
          data-test="grid-menu-copy-without-names"
          @click="copyRowsOnly"
        />
        <v-list-item
          title="View full value"
          data-test="grid-menu-inspect"
          @click="inspectCellOfMenu"
        />
      </v-list>
    </v-menu>

    <AppDialog v-model="inspecting" max-width="720">
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
    </AppDialog>
  </div>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  shallowRef,
  triggerRef,
  watch,
} from 'vue'
import type { ComponentPublicInstance } from 'vue'
import PanelHeader from './PanelHeader.vue'
import { compareSortKeys, formatCell, isNullCell, sortKey, truncate } from '@/lib/format'
import type { SortKey } from '@/lib/format'
import { toTabSeparated } from '@/lib/export'
import type { ResultTable } from '@/lib/results'
import type { CellValue, ResultSet } from '@/types/api'

/** The forms an export can take. */
export type ExportFormat = 'csv' | 'json' | 'markdown' | 'insert' | 'xlsx'

/**
 * The forms the export of every row can take. The backend writes those
 * files one row at a time, and it holds no writer for the other forms.
 */
export type ExportAllFormat = 'csv' | 'json' | 'xlsx'

const props = withDefaults(
  defineProps<{
    result: ResultTable
    /** The rows the table holds. The table stands outside the reactivity of
     *  Vue, so this count tells the grid that rows arrived while the set
     *  streams. A grid without it reads the count of the table once. */
    rows?: number
    /** True when the row limit stopped the read. The table sets its mark
     *  when the set ends, and Vue does not see that change, so the pane
     *  gives it here. A grid without it reads the mark of the table once. */
    truncated?: boolean
    busy?: boolean
  }>(),
  { busy: false, rows: undefined, truncated: undefined },
)

/** The number of rows of the result, which grows while the set streams. */
const rowTotal = computed(() => props.rows ?? props.result.rowCount)
/** True when the row limit stopped the read of the result. */
const truncated = computed(() => props.truncated ?? props.result.truncated)
const emit = defineEmits<{
  (event: 'export', format: ExportFormat, rows: ResultSet): void
  (event: 'export-all', format: ExportAllFormat): void
  (event: 'copied', text: string): void
}>()

/** The height of one row, which the window of visible rows is built from. */
const ROW_HEIGHT = 30
/** The number of rows drawn above and below the visible area. */
const OVERSCAN = 12
/** The number of letters of a value that one cell holds. */
const CELL_LIMIT = 160

const search = ref('')
/**
 * The filter text the rows are matched against. It follows the field after
 * a short pause, so that a keystroke does not scan every row at once.
 */
const appliedSearch = ref('')
/**
 * The result that the user set the filter for. A new result starts with no
 * filter, but the watch that clears the filter runs after the watch that
 * reads the text of the rows. Without this check, a switch to a new result
 * reads one slice of its rows for the filter of the result that has gone.
 */
let filterSource: ResultTable | null = null
const FILTER_DELAY_MS = 200
let filterTimer: ReturnType<typeof setTimeout> | null = null
watch(search, (value) => {
  if (filterTimer !== null) {
    clearTimeout(filterTimer)
  }
  filterTimer = setTimeout(() => {
    filterTimer = null
    filterSource = props.result
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
/** The header row, which holds still at the top of the area above the rows. */
const headerRow = ref<HTMLElement | null>(null)
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
  dropRowTexts()
  stopSortTimer()
  // A drag of a grip that runs at unmount leaves its listeners on the window.
  endResize()
})

const inspecting = ref(false)
const inspectValue = ref('')
const inspectTitle = ref('')

/**
 * The rows the user selected, held by their place in the result and not by
 * their place in the view. A sort or a filter moves a row in the view, and
 * the selection must follow the row and not the position.
 *
 * Each change gives a new set. The set stays outside the deep reactivity of
 * Vue, so a selection of every row of a large result costs no proxy and no
 * tracked read for each row.
 */
const selected = shallowRef(new Set<number>())
/** The row of the last plain click, from which a click with Shift reaches. */
const anchor = ref<number | null>(null)

/**
 * The place of every row of the result, in the order of the result. The view
 * holds places and not rows, so a result of many rows costs one number for
 * each row and no object. A chunk that arrives while the set streams adds the
 * places of its own rows to the end of the same array, so the rows of a set
 * of many chunks are counted once and not once for each chunk.
 */
let sourceOrder: number[] = []
let sourceOrderTable: ResultTable | null = null

function sourceOrderFor(table: ResultTable, total: number): number[] {
  if (sourceOrderTable !== table || sourceOrder.length > total) {
    sourceOrder = []
    sourceOrderTable = table
  }
  for (let index = sourceOrder.length; index < total; index += 1) {
    sourceOrder.push(index)
  }
  return sourceOrder
}

/**
 * The text of every row in small letters, which the filter matches against.
 * The copy weighs as much as the result itself, so it lives only while a
 * filter is active: it is built when the first filter arrives, kept between
 * keystrokes, and released when the filter clears or the result changes.
 *
 * A result of many rows takes longer to read than one frame, so the build
 * runs in slices and gives the main thread back between them. The rows on
 * screen stay under the filter that the finished text answers, and the
 * header says that a build runs.
 */
let rowTexts: string[] = []
let rowTextsSource: ResultTable | null = null
/** Rises when a slice of the build finished the text of every row. */
const rowTextsVersion = ref(0)
/** The filter the finished text answers, which the rows on screen follow. */
const activeFilter = ref('')
/** The part of the rows the build covered, or null while no build runs. */
const filterProgress = ref<number | null>(null)

/**
 * The words for a table with no row on show. A filter that matches no row
 * is not a statement that gave no row, so the two states read apart.
 */
const emptyMessage = computed(() =>
  rowTotal.value > 0 ? 'No rows match the filter.' : 'This statement returned no rows.',
)
/** The longest a slice of the build holds the main thread. */
const BUILD_SLICE_MS = 12
/** The most rows one slice reads, so a slice of small rows also gives way. */
const BUILD_SLICE_ROWS = 5000
let buildTimer: ReturnType<typeof setTimeout> | null = null

function rowText(table: ResultTable, index: number): string {
  return table
    .row(index)
    .map((cell) => formatCell(cell))
    .join(' ')
    .toLowerCase()
}

/** Holds back the slice of a build that waits for its turn. */
function stopBuild(): void {
  if (buildTimer !== null) {
    clearTimeout(buildTimer)
    buildTimer = null
  }
}

/** Drops the text and stops a build that runs. */
function dropRowTexts(): void {
  stopBuild()
  rowTexts = []
  rowTextsSource = null
  filterProgress.value = null
  matches = []
  matchesSource = null
}

/**
 * Reads one slice of the rows the text does not hold yet. It calls itself
 * through a timer until the text covers every row, and it then puts the
 * filter of the field on the rows.
 */
function buildRowTexts(): void {
  buildTimer = null
  const table = props.result
  const total = rowTotal.value
  const start = performance.now()
  let read = 0
  while (
    rowTexts.length < total &&
    read < BUILD_SLICE_ROWS &&
    (read === 0 || performance.now() - start < BUILD_SLICE_MS)
  ) {
    rowTexts.push(rowText(table, rowTexts.length))
    read += 1
  }
  if (rowTexts.length < total) {
    filterProgress.value = rowTexts.length / total
    buildTimer = setTimeout(buildRowTexts, 0)
    return
  }
  filterProgress.value = null
  activeFilter.value = appliedSearch.value
  rowTextsVersion.value += 1
}

/**
 * Makes the text of the rows answer the filter of the field. The text of a
 * row that the build already read is kept, so a row that arrived while the
 * set streams costs its own text alone.
 */
function refreshRowTexts(): void {
  if (appliedSearch.value === '' || filterSource !== props.result) {
    dropRowTexts()
    activeFilter.value = ''
    return
  }
  if (rowTextsSource !== props.result) {
    dropRowTexts()
    rowTextsSource = props.result
  }
  stopBuild()
  // The first slice runs now, so a small result answers the filter in the
  // same tick and draws no mark of a build.
  buildRowTexts()
}

watch([appliedSearch, rowTotal, () => props.result], refreshRowTexts)

/**
 * The places of the rows that match the active filter. The text of a row that
 * arrives while the filter stands is matched once, and its place goes on the
 * end of the array.
 */
let matches: number[] = []
let matchesSource: ResultTable | null = null
let matchesNeedle = ''
/** The number of row texts that the matches cover. */
let matchesRead = 0

function matchesFor(table: ResultTable, needle: string): number[] {
  if (matchesSource !== table || matchesNeedle !== needle) {
    matches = []
    matchesSource = table
    matchesNeedle = needle
    matchesRead = 0
  }
  // The texts of a result that has gone stay until the build for the new
  // result starts, and they do not name the rows of the new result.
  const texts = rowTextsSource === table ? rowTexts : []
  for (; matchesRead < texts.length; matchesRead += 1) {
    if (texts[matchesRead]!.includes(needle)) {
      matches.push(matchesRead)
    }
  }
  return matches
}

/**
 * The value of one column for every row, which the sort compares. The keys
 * are built once for a column, so a sort of many rows builds the text of a
 * cell once and not once for each comparison. The keys of rows that arrive
 * while the set streams go on the end of the same array. The keys go when
 * the sort clears.
 */
let sortKeys: SortKey[] = []
let sortKeysSource: ResultTable | null = null
let sortKeysColumn = -1

function sortKeysFor(table: ResultTable, column: number, total: number): SortKey[] {
  if (sortKeysSource !== table || sortKeysColumn !== column || sortKeys.length > total) {
    sortKeys = []
    sortKeysSource = table
    sortKeysColumn = column
  }
  for (let index = sortKeys.length; index < total; index += 1) {
    sortKeys.push(sortKey(table.cell(index, column)))
  }
  return sortKeys
}

/** The places of the rows in the view, after the filter and the sort. */
const sortedOrder = shallowRef<number[]>([])
/** The rows that the order of the view covers. */
let orderedCount = 0
/** The time that the last sort took, and the time at which it ended. */
let lastSortMs = 0
let lastSortEnd = Number.NEGATIVE_INFINITY
/**
 * A sort after new rows waits until this many times the time of the last sort
 * has passed since that sort ended. The sort of a set that streams thus takes
 * at most about a fifth of the main thread. While it waits, the view shows
 * the rows of the last sort, and the count names the rows that the view holds.
 */
const SORT_GAP_FACTOR = 4
let sortTimer: ReturnType<typeof setTimeout> | null = null

function stopSortTimer(): void {
  if (sortTimer !== null) {
    clearTimeout(sortTimer)
    sortTimer = null
  }
}

/** The places of the rows that the filter keeps, in the order of the result. */
function baseOrder(): number[] {
  const needle = activeFilter.value
  return needle === ''
    ? sourceOrderFor(props.result, rowTotal.value)
    : matchesFor(props.result, needle)
}

/**
 * The result that the user chose the sort for. A new result starts with no
 * sort, but the watch that clears the sort runs after the order is built
 * again. Without this check, a switch to a large result sorts every row of
 * it once by the sort of the result that has gone.
 */
let sortSource: ResultTable | null = null

/** Builds the order of the view again, with the sort when one is active. */
function updateOrder(): void {
  stopSortTimer()
  const base = baseOrder()
  orderedCount = base.length
  const index = sortSource === props.result ? sortIndex.value : null
  if (index === null) {
    // The keys weigh as much as one column of the result, so they go with
    // the sort. A sort of the same column later builds them again.
    sortKeys = []
    sortKeysSource = null
    // The array of the places can be the one that the view holds already, so
    // the view is told of its new rows.
    sortedOrder.value = base
    triggerRef(sortedOrder)
    return
  }
  const start = performance.now()
  const direction = sortDescending.value ? -1 : 1
  const keys = sortKeysFor(props.result, index, rowTotal.value)
  sortedOrder.value = [...base].sort(
    (left, right) => compareSortKeys(keys[left] ?? null, keys[right] ?? null) * direction,
  )
  lastSortEnd = performance.now()
  lastSortMs = lastSortEnd - start
}

/**
 * Adds rows that arrived to the order of the view. Without a sort this costs
 * the new rows alone. With a sort, a sort that ended a short time ago makes
 * the next one wait, so a set of many chunks is not sorted again for each
 * chunk.
 */
function updateOrderLater(): void {
  if (sortTimer !== null || baseOrder().length === orderedCount) {
    return
  }
  const wait = lastSortEnd + lastSortMs * SORT_GAP_FACTOR - performance.now()
  if (sortIndex.value === null || wait <= 0) {
    updateOrder()
    return
  }
  sortTimer = setTimeout(updateOrder, wait)
}

watch([() => props.result, activeFilter, sortIndex, sortDescending], updateOrder, {
  flush: 'sync',
  immediate: true,
})
watch([rowTotal, rowTextsVersion], updateOrderLater, { flush: 'sync' })

const hasSelection = computed(() => selected.value.size > 0)

const exportItems = computed<Array<{ format: ExportFormat; title: string }>>(() => {
  const scope = hasSelection.value ? 'selected rows' : 'rows'
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
  Math.min(sortedOrder.value.length, firstVisible.value + visibleCount.value),
)

// The rows of the window are the only rows that stand as arrays of values.
const windowRows = computed(() =>
  sortedOrder.value.slice(firstVisible.value, lastVisible.value).map((sourceIndex, offset) => {
    const row = props.result.row(sourceIndex)
    return {
      sourceIndex,
      row,
      position: firstVisible.value + offset,
      // The text of each cell is built once here, so the view does not build
      // it again for the tooltip and for the body of the cell.
      texts: row.map((cell) => formatCell(cell)),
    }
  }),
)

const topPad = computed(() => firstVisible.value * ROW_HEIGHT)
const bottomPad = computed(() =>
  Math.max(0, (sortedOrder.value.length - lastVisible.value) * ROW_HEIGHT),
)

const countLabel = computed(() => {
  const shown = sortedOrder.value.length
  const total = rowTotal.value
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
  sortSource = props.result
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
 * The cell that carries the one tab stop of the grid, given as the place of its
 * row among the sorted rows and the place of its column. A grid of ten thousand
 * rows would otherwise hold a tab stop for every cell it draws.
 */
const focusedRow = ref(0)
const focusedColumn = ref(0)
const cellElements = new Map<string, HTMLElement>()

function cellId(row: number, column: number): string {
  return `${row}:${column}`
}

function keepCell(
  row: number,
  column: number,
  element: Element | ComponentPublicInstance | null,
): void {
  const id = cellId(row, column)
  if (element === null) {
    cellElements.delete(id)
    return
  }
  cellElements.set(id, element as HTMLElement)
}

function isFocused(entry: { position: number }, column: number): boolean {
  return focusedRow.value === entry.position && focusedColumn.value === column
}

/**
 * Puts the tab stop on one cell. The grid draws only the rows near the visible
 * part, so a cell outside that part is first scrolled into it and the focus
 * follows once the row is drawn.
 */
function focusCellAt(row: number, column: number, move = true): void {
  const lastRow = Math.max(0, sortedOrder.value.length - 1)
  const lastColumn = Math.max(0, props.result.columns.length - 1)
  focusedRow.value = Math.min(lastRow, Math.max(0, row))
  focusedColumn.value = Math.min(lastColumn, Math.max(0, column))
  if (!move) {
    return
  }
  scrollRowIntoView(focusedRow.value)
  void nextTick(() => {
    const cell = cellElements.get(cellId(focusedRow.value, focusedColumn.value))
    if (!cell) {
      return
    }
    // The scroll above already put the row in the visible part, and the browser
    // would move it again behind the header. The second call moves the area
    // sideways alone, because the row stands inside it in the other direction.
    cell.focus({ preventScroll: true })
    cell.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
}

/**
 * Scrolls the area so that one row stands inside it, below the header that
 * holds still at the top.
 */
function scrollRowIntoView(row: number): void {
  const area = scrollArea.value
  if (!area) {
    return
  }
  const top = row * ROW_HEIGHT
  const bottom = top + ROW_HEIGHT
  const header = headerRow.value?.clientHeight ?? 0
  let next = area.scrollTop
  if (top - header < next) {
    next = top - header
  } else if (bottom > next + area.clientHeight) {
    next = bottom - area.clientHeight
  }
  next = Math.max(0, next)
  if (next !== area.scrollTop) {
    area.scrollTop = next
  }
  // The scroll event of the area arrives after the next frame, and the rows
  // that the grid draws follow this value. The focus needs the row now, so the
  // value moves with the area and not with the event.
  scrollTop.value = area.scrollTop
}

/** The number of whole rows the visible part of the area holds. */
function rowsPerPage(): number {
  return Math.max(1, Math.floor(viewportHeight.value / ROW_HEIGHT))
}

function onGridKeyDown(event: KeyboardEvent): void {
  // A key of a sort button or of a grip belongs to that control. Without
  // this check, Enter on a sort button would also open the value of a cell,
  // and an arrow on a grip would also move the focus into the rows.
  const target = event.target as Element | null
  if (sortedOrder.value.length === 0 || target?.closest('thead')) {
    return
  }
  const row = focusedRow.value
  const column = focusedColumn.value
  const lastRow = sortedOrder.value.length - 1
  const lastColumn = props.result.columns.length - 1

  switch (event.key) {
    case 'ArrowDown':
      focusCellAt(row + 1, column)
      break
    case 'ArrowUp':
      focusCellAt(row - 1, column)
      break
    case 'ArrowRight':
      focusCellAt(row, column + 1)
      break
    case 'ArrowLeft':
      focusCellAt(row, column - 1)
      break
    case 'PageDown':
      focusCellAt(row + rowsPerPage(), column)
      break
    case 'PageUp':
      focusCellAt(row - rowsPerPage(), column)
      break
    case 'Home':
      // Control reaches the first cell of the whole grid, and the key alone
      // reaches the first cell of the row.
      focusCellAt(event.ctrlKey || event.metaKey ? 0 : row, 0)
      break
    case 'End':
      focusCellAt(event.ctrlKey || event.metaKey ? lastRow : row, lastColumn)
      break
    case 'Enter':
      openFocusedCell()
      break
    case ' ':
      toggleFocusedRow(event)
      break
    case 'a':
    case 'A':
      if (!event.ctrlKey && !event.metaKey) {
        return
      }
      selectAllRows()
      break
    case 'ContextMenu':
      openFocusedCellMenu()
      break
    case 'F10':
      if (!event.shiftKey) {
        return
      }
      openFocusedCellMenu()
      break
    default:
      return
  }
  event.preventDefault()
}

/** The place in the result of the row the tab stop stands on. */
function focusedSourceRow(): number | undefined {
  return sortedOrder.value[focusedRow.value]
}

/** Takes every row of the view, as the filter leaves them. */
function selectAllRows(): void {
  selected.value = new Set(sortedOrder.value)
  anchor.value = 0
}

/**
 * Answers the copy of the browser, which Ctrl+C, Cmd+C and the Edit menu all
 * send. The Edit menu of macOS takes Cmd+C before the page sees the key, so a
 * handler of the key alone would never run there. Text that the user marked
 * with the pointer is copied as the browser copies it. Otherwise the selected
 * rows go to the clipboard, and with no selection the cell of the tab stop
 * goes there.
 */
function onGridCopy(event: ClipboardEvent): void {
  if (globalThis.getSelection?.()?.toString()) {
    return
  }
  const cell = cellAt(focusedRow.value, focusedColumn.value)
  if (cell === undefined) {
    return
  }
  const text = hasSelection.value ? toTabSeparated(rowsToExport().rows) : formatCell(cell)
  event.preventDefault()
  if (event.clipboardData) {
    event.clipboardData.setData('text/plain', text)
    emit('copied', text)
  } else {
    void copyText(text)
  }
}

/** Opens the whole value of the cell the tab stop stands on. */
function openFocusedCell(): void {
  const row = focusedSourceRow()
  if (row !== undefined) {
    inspect(props.result.cell(row, focusedColumn.value), columnName(focusedColumn.value))
  }
}

/**
 * Takes the row the tab stop stands on, or adds it to the rows already taken
 * when Control or Command is held.
 */
function toggleFocusedRow(event: KeyboardEvent): void {
  const row = focusedSourceRow()
  if (row !== undefined) {
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(selected.value)
      if (next.has(row)) {
        next.delete(row)
      } else {
        next.add(row)
      }
      selected.value = next
    } else {
      selected.value = new Set([row])
    }
    anchor.value = focusedRow.value
  }
}

/**
 * Puts the whole value of a cell under the pointer, and only when the cell is
 * too narrow to show all of it. How much a cell shows is known to the browser
 * alone, so the question is asked when the pointer or the focus arrives, and
 * not while the row is drawn.
 */
function revealFullValue(event: Event, row: number, column: number): void {
  const cell = event.currentTarget as HTMLElement
  const text = cell.querySelector('.cell-text')
  if (!text || text.scrollWidth <= text.clientWidth) {
    cell.removeAttribute('title')
    return
  }
  const source = sortedOrder.value[row]
  cell.title = source === undefined ? '' : formatCell(props.result.cell(source, column))
}

function onCellFocus(event: Event, row: number, column: number): void {
  focusCellAt(row, column, false)
  revealFullValue(event, row, column)
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
    for (const source of sortedOrder.value.slice(from, to + 1)) {
      next.add(source)
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
  const order = hasSelection.value
    ? sortedOrder.value.filter((row) => selected.value.has(row))
    : sortedOrder.value
  // An export builds the rows it writes, and it builds them at the moment the
  // user asks for the file.
  return {
    columns: props.result.columns,
    rows: order.map((row) => props.result.row(row)),
    truncated: truncated.value,
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

/** Copies the rows alone, with no line of column names above them. */
function copyRowsOnly(): void {
  void copyText(toTabSeparated(rowsToExport().rows))
}

/** The cell of one place of the view, or nothing when the place is empty. */
function cellAt(position: number, column: number): CellValue | undefined {
  const source = sortedOrder.value[position]
  return source === undefined ? undefined : props.result.cell(source, column)
}

/** Copies the value of one cell as text. */
function copyCell(position: number, column: number): void {
  const cell = cellAt(position, column)
  if (cell !== undefined) {
    void copyText(formatCell(cell))
  }
}

/** Copies one row of the view as text. */
function copyRow(position: number): void {
  const source = sortedOrder.value[position]
  if (source !== undefined) {
    void copyText(toTabSeparated([props.result.row(source)]))
  }
}

/** The place the menu of the cells stands at, and the cell it belongs to. */
const cellMenu = ref({ open: false, x: 0, y: 0, row: 0, column: 0 })
/** True while the menu stands open because of a key and not the pointer. */
let menuFromKeys = false

/** Opens the menu of one cell where the pointer stands. */
function openCellMenu(event: MouseEvent, position: number, column: number): void {
  focusCellAt(position, column)
  cellMenu.value = { open: true, x: event.clientX, y: event.clientY, row: position, column }
  menuFromKeys = false
}

/**
 * Opens the menu of the cell of the tab stop, below that cell. Shift+F10 and
 * the menu key reach it, so a user of the keyboard alone gets the same menu
 * as the pointer.
 */
function openFocusedCellMenu(): void {
  const row = focusedRow.value
  const column = focusedColumn.value
  const cell = cellElements.get(cellId(row, column))
  if (!cell) {
    return
  }
  const box = cell.getBoundingClientRect()
  cellMenu.value = { open: true, x: box.left, y: box.bottom, row, column }
  menuFromKeys = true
}

// The menu opens at a point and has no element that opened it. When a menu
// that a key opened closes, the focus goes back to its cell. The focus moves
// before the dialog of a value opens. The dialog then keeps the cell as the
// element that opened it, and it gives the focus back to the cell.
watch(
  () => cellMenu.value.open,
  (open) => {
    if (open || !menuFromKeys) {
      return
    }
    menuFromKeys = false
    cellElements.get(cellId(cellMenu.value.row, cellMenu.value.column))?.focus()
  },
)

function copyCellOfMenu(): void {
  copyCell(cellMenu.value.row, cellMenu.value.column)
}

function copyRowOfMenu(): void {
  copyRow(cellMenu.value.row)
}

function inspectCellOfMenu(): void {
  const cell = cellAt(cellMenu.value.row, cellMenu.value.column)
  if (cell !== undefined) {
    inspect(cell, columnName(cellMenu.value.column))
  }
}

/**
 * The width the user gave each column, by the place of the column. A column
 * that the user never dragged holds no width and takes the width of its
 * content.
 */
const columnWidths = ref<Record<number, number>>({})
/** The narrowest a column can become. */
const MIN_COLUMN_WIDTH = 56
/** The step of a change of the width from the keyboard. */
const WIDTH_STEP = 16
/** The drag that runs, when one runs. */
let resizing: { column: number; startX: number; startWidth: number } | null = null

function headerStyle(index: number): Record<string, string> {
  const width = columnWidths.value[index]
  return width === undefined
    ? {}
    : { width: `${width}px`, minWidth: `${width}px`, maxWidth: `${width}px` }
}

function cellStyle(index: number): Record<string, string> {
  const width = columnWidths.value[index]
  return width === undefined ? {} : { maxWidth: `${width}px` }
}

/** Gives one column a width, never narrower than the limit. */
function setWidth(index: number, width: number): void {
  columnWidths.value = { ...columnWidths.value, [index]: Math.max(MIN_COLUMN_WIDTH, width) }
}

/** Lets a column take the width of its content again. */
function clearWidth(index: number): void {
  const next = { ...columnWidths.value }
  delete next[index]
  columnWidths.value = next
}

/** The width one column holds now, from the record or from the element. */
function widthOf(index: number, element: HTMLElement | null): number {
  return columnWidths.value[index] ?? element?.getBoundingClientRect().width ?? MIN_COLUMN_WIDTH
}

/** Follows the pointer while it drags the grip of a column. */
function onResizeMove(event: PointerEvent): void {
  if (resizing) {
    setWidth(resizing.column, resizing.startWidth + (event.clientX - resizing.startX))
  }
}

/** Ends the drag of a grip. */
function endResize(): void {
  resizing = null
  globalThis.removeEventListener('pointermove', onResizeMove)
  globalThis.removeEventListener('pointerup', endResize)
}

/** Starts the drag of the grip of one column. */
function startResize(event: PointerEvent, index: number): void {
  const header = (event.target as HTMLElement | null)?.parentElement ?? null
  resizing = { column: index, startX: event.clientX, startWidth: widthOf(index, header) }
  globalThis.addEventListener('pointermove', onResizeMove)
  globalThis.addEventListener('pointerup', endResize)
  event.preventDefault()
}

/**
 * The keys of the grip of a column. The arrows change the width by one step,
 * and Enter lets the column take the width of its content again.
 */
function onGripKeyDown(event: KeyboardEvent, index: number): void {
  const header = (event.target as HTMLElement | null)?.parentElement ?? null
  switch (event.key) {
    case 'ArrowRight':
      setWidth(index, widthOf(index, header) + WIDTH_STEP)
      break
    case 'ArrowLeft':
      setWidth(index, widthOf(index, header) - WIDTH_STEP)
      break
    case 'Enter':
      clearWidth(index)
      break
    default:
      return
  }
  event.preventDefault()
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
    columnWidths.value = {}
    // The tab stop returns to the first cell, because the rows it stood on
    // belong to the result that has gone.
    focusedRow.value = 0
    focusedColumn.value = 0
    cellElements.clear()
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

.grid-body {
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.grid-scroll {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
}

/* The cover lies over the rows of the result that has gone. It lets the eye
   read them and stops a click from acting on them. */
.grid-busy {
  position: absolute;
  inset: 0;
  z-index: 3;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: rgba(var(--v-theme-surface), 0.6);
}

.focused-cell:focus-visible,
.focused-cell:focus {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
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
  /* A column that the user made narrow cuts its own name. The grip of the
     column sits against the right edge of this element, because a sticky
     element also places its children that have an absolute position. */
  overflow: hidden;
}

/* The grip sits on the right edge of the header of a column. */
.column-grip {
  position: absolute;
  top: 0;
  right: 0;
  width: 6px;
  height: 100%;
  cursor: col-resize;
  touch-action: none;
}

.column-grip:hover,
.column-grip:focus-visible {
  background: rgba(var(--v-theme-primary), 0.4);
  outline: none;
}

.header-button {
  display: block;
  width: 100%;
  overflow: hidden;
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
  border-bottom: var(--app-divider-soft);
  cursor: default;
}

/* One wide column would push every column after it off the screen, so a cell
   shows this much of its value and no more. The whole value waits under the
   pointer and in the window that the Enter key opens. */
.cell-text {
  display: block;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
