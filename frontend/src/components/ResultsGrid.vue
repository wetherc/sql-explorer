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
              <v-list-item
                title="Write every row to an Excel file"
                subtitle="Runs the statement again and writes from the server"
                data-test="grid-export-all-xlsx"
                @click="emit('export-all', 'xlsx')"
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
        The row limit stopped the read at {{ result.rowCount.toLocaleString() }} rows.
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
          aria-label="The rows of the result"
          :aria-rowcount="sortedOrder.length + 1"
          :aria-colcount="result.columns.length + 1"
          :aria-busy="busy"
          @keydown="onGridKeyDown"
        >
          <thead>
            <tr role="row" aria-rowindex="1">
              <th class="row-number" role="columnheader" aria-colindex="1" scope="col">#</th>
              <th
                v-for="(column, index) in result.columns"
                :key="`${column.name}-${index}`"
                :class="{ sorted: sortIndex === index }"
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
              >
                <!-- The width of a cell is capped on this element and not on
                     the cell itself, because a table of automatic width pays
                     no attention to a cap on one of its cells. -->
                <span class="cell-text">
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
              >
                This statement returned no rows.
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

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
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { ComponentPublicInstance } from 'vue'
import PanelHeader from './PanelHeader.vue'
import { compareCells, formatCell, isNullCell, truncate } from '@/lib/format'
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

const props = withDefaults(defineProps<{ result: ResultTable; busy?: boolean }>(), { busy: false })
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

/**
 * The place of every row of the result. The view holds places and not rows,
 * so a result of many rows costs one number for each row and no object.
 */
const sourceOrder = computed(() => {
  const order = new Array<number>(props.result.rowCount)
  for (let index = 0; index < order.length; index += 1) {
    order[index] = index
  }
  return order
})

/**
 * The text of every row in small letters, which the filter matches against.
 * The copy weighs as much as the result itself, so it lives only while a
 * filter is active: it is built when the first filter arrives, kept between
 * keystrokes, and released when the filter clears or the result changes.
 */
let rowTexts: string[] | null = null
let rowTextsSource: ResultTable | null = null

function rowTextsFor(table: ResultTable): string[] {
  if (rowTexts === null || rowTextsSource !== table) {
    const texts = new Array<string>(table.rowCount)
    for (let index = 0; index < texts.length; index += 1) {
      texts[index] = table
        .row(index)
        .map((cell) => formatCell(cell))
        .join(' ')
        .toLowerCase()
    }
    rowTexts = texts
    rowTextsSource = table
  }
  return rowTexts
}

watch(appliedSearch, (needle) => {
  if (needle === '') {
    rowTexts = null
    rowTextsSource = null
  }
})

const filteredOrder = computed(() => {
  const needle = appliedSearch.value
  if (needle === '') {
    return sourceOrder.value
  }
  const texts = rowTextsFor(props.result)
  return sourceOrder.value.filter((row) => texts[row]?.includes(needle))
})

const sortedOrder = computed(() => {
  const index = sortIndex.value
  if (index === null) {
    return filteredOrder.value
  }
  const direction = sortDescending.value ? -1 : 1
  const table = props.result
  return [...filteredOrder.value].sort(
    (left, right) => compareCells(table.cell(left, index), table.cell(right, index)) * direction,
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
  const total = props.result.rowCount
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
  void nextTick(() => cellElements.get(cellId(focusedRow.value, focusedColumn.value))?.focus())
}

/** Scrolls the area so that one row stands inside it. */
function scrollRowIntoView(row: number): void {
  const area = scrollArea.value
  if (area) {
    const top = row * ROW_HEIGHT
    const bottom = top + ROW_HEIGHT
    if (top < area.scrollTop) {
      area.scrollTop = top
    } else if (bottom > area.scrollTop + area.clientHeight) {
      area.scrollTop = bottom - area.clientHeight
    }
  }
}

/** The number of whole rows the visible part of the area holds. */
function rowsPerPage(): number {
  return Math.max(1, Math.floor(viewportHeight.value / ROW_HEIGHT))
}

function onGridKeyDown(event: KeyboardEvent): void {
  if (sortedOrder.value.length === 0) {
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
    default:
      return
  }
  event.preventDefault()
}

/** The place in the result of the row the tab stop stands on. */
function focusedSourceRow(): number | undefined {
  return sortedOrder.value[focusedRow.value]
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
