<template>
  <div
    ref="scrollArea"
    class="explorer-scroll"
    role="tree"
    aria-label="Database objects"
    :aria-busy="busy"
    :tabindex="activeDrawn ? -1 : 0"
    :style="{ '--tree-row-height': `${ROW_HEIGHT}px` }"
    @keydown="onKeyDown"
    @scroll="onScroll"
    @focus.self="onAreaFocus"
  >
    <!-- The tree scrolls in the element above, and it draws only the rows
         around the visible part. Two empty blocks hold the space of the rows
         above and below, so the bar of the scroll answers for the whole
         tree. The block here carries the width of the widest row, drawn or
         not. It stands outside the reading, so the rows belong to the tree
         itself. -->
    <div
      class="explorer-tree"
      role="presentation"
      :style="{ '--tree-width': `${treeWidth}px` }"
      data-test="tree-body"
    >
      <div v-if="topPad > 0" :style="{ height: `${topPad}px` }" aria-hidden="true"></div>
      <template v-for="row in windowRows" :key="row.key">
        <div
          v-if="row.rowType === 'node'"
          :ref="(element) => keepRow(row.key, element)"
          class="tree-row"
          :class="{ selected: selectedKey === row.key, dimmed: row.node.dimmed }"
          :style="{ paddingLeft: rowIndent(row.depth) }"
          role="treeitem"
          :aria-level="row.depth + 1"
          :aria-posinset="row.posInSet"
          :aria-setsize="row.setSize"
          :aria-expanded="row.expandable ? row.expanded : undefined"
          :aria-selected="selectedKey === row.key"
          :tabindex="activeKey === row.key ? 0 : -1"
          data-test="tree-row"
          @click="activate(row.node)"
          @focus="focusedKey = row.key"
          @contextmenu.prevent="openMenuAt($event.clientX, $event.clientY, row.node)"
        >
          <v-icon
            v-if="row.expandable"
            size="x-small"
            class="chevron"
            aria-hidden="true"
            data-test="tree-chevron"
          >
            {{ row.expanded ? 'mdi-chevron-down' : 'mdi-chevron-right' }}
          </v-icon>
          <span v-else class="chevron-space"></span>

          <v-progress-circular
            v-if="row.node.loading"
            indeterminate
            size="12"
            width="2"
            class="mr-2"
            data-test="tree-loading"
          />
          <v-icon v-else size="small" class="mr-2 node-icon" aria-hidden="true">
            {{ row.node.icon }}
          </v-icon>

          <span class="node-label">{{ row.node.label }}</span>
          <span v-if="row.node.hint" class="node-hint">{{ row.node.hint }}</span>
        </div>

        <div
          v-else-if="row.rowType === 'error'"
          class="empty-branch error-branch"
          :style="{ paddingLeft: labelIndent(row.depth) }"
          role="treeitem"
          :aria-level="row.depth + 1"
          aria-disabled="true"
          tabindex="-1"
          data-test="tree-error"
        >
          <span class="error-text" :title="row.message">Couldn't load: {{ row.message }}</span>
          <v-btn
            size="x-small"
            variant="text"
            color="primary"
            text="Retry"
            data-test="tree-retry"
            @click.stop="emit('retry', row.node)"
          />
        </div>

        <div
          v-else
          class="empty-branch"
          :style="{ paddingLeft: labelIndent(row.depth) }"
          role="treeitem"
          :aria-level="row.depth + 1"
          aria-disabled="true"
          tabindex="-1"
          data-test="tree-empty"
        >
          Nothing here
        </div>
      </template>
      <div v-if="bottomPad > 0" :style="{ height: `${bottomPad}px` }" aria-hidden="true"></div>
    </div>
  </div>
</template>

<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  onUpdated,
  ref,
  type ComponentPublicInstance,
} from 'vue'
import { isExpandable, type ExplorerNode } from '@/stores/explorer'
import { canvasContext, createTextMeter, fontOf } from '@/lib/textWidth'

/** The width of one step of the indent. */
const INDENT_STEP = 14
/** The space to the left of the chevron of a row. */
const ROW_PADDING = 6
/**
 * The distance from the left of a row to its label. It counts the chevron, the
 * gap after it, the icon and the margin of the icon, so a line that stands in
 * the place of a child begins under the label of a child.
 */
const LABEL_OFFSET = 46

/** The space between a label and its hint: the gap of the row and the pad of the hint. */
const HINT_GAP = 10
/** The space to the right of the last part of a row. */
const ROW_END = 8

/** How long a type-ahead holds its letters before it starts again. */
const TYPE_AHEAD_MS = 800

/** The height of one row, which the window of drawn rows is built from. */
const ROW_HEIGHT = 24
/** The number of rows drawn above and below the visible part. */
const OVERSCAN = 10

const props = withDefaults(
  defineProps<{
    nodes: ExplorerNode[]
    openKeys: Set<string>
    selectedKey?: string | null
    /** True while a read of the tree runs, which the store counts. */
    busy?: boolean
  }>(),
  { selectedKey: null, busy: false },
)

const emit = defineEmits<{
  (event: 'activate', node: ExplorerNode): void
  (event: 'expand', node: ExplorerNode): void
  (event: 'collapse', node: ExplorerNode): void
  (event: 'retry', node: ExplorerNode): void
  (event: 'context', payload: { x: number; y: number; node: ExplorerNode }): void
}>()

/** One line of the tree: a node, the note of a branch with none, or the note of a failed read. */
type Row =
  | {
      rowType: 'node'
      key: string
      node: ExplorerNode
      depth: number
      posInSet: number
      setSize: number
      expandable: boolean
      expanded: boolean
    }
  | { rowType: 'empty'; key: string; depth: number }
  | { rowType: 'error'; key: string; depth: number; node: ExplorerNode; message: string }

/** The row that holds the focus, which is the one row the Tab key reaches. */
const focusedKey = ref<string | null>(null)
const rowElements = new Map<string, HTMLElement>()

/** The element the rows scroll in, and the place and the height of it. */
const scrollArea = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportHeight = ref(600)
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

onBeforeUnmount(() => {
  sizeObserver?.disconnect()
  sizeObserver = null
})

let typed = ''
let typedAt = 0

/**
 * The rows of the tree, in the order the eye and the keys move through them.
 *
 * The tree draws one flat list and gives each row its level, because the keys
 * move between the rows the user can see and that order is what a flat list
 * holds. A reader builds the shape of the tree from the levels.
 */
const rows = computed<Row[]>(() => {
  const out: Row[] = []
  const walk = (nodes: ExplorerNode[], depth: number): void => {
    nodes.forEach((node, index) => {
      const expandable = isExpandable(node)
      const expanded = props.openKeys.has(node.key)
      out.push({
        rowType: 'node',
        key: node.key,
        node,
        depth,
        posInSet: index + 1,
        setSize: nodes.length,
        expandable,
        expanded,
      })
      if (!expanded) {
        return
      }
      const children = node.children ?? []
      // The store clears the failure as a read starts, so the rows do not
      // read the mark of the read. A start or an end of a read then builds
      // no row again and measures no width again.
      const failure = node.error ?? null
      if (children.length > 0) {
        walk(children, depth + 1)
      } else if (failure) {
        out.push({
          rowType: 'error',
          key: `${node.key} error`,
          depth: depth + 1,
          node,
          message: failure,
        })
      } else if (node.loaded) {
        out.push({ rowType: 'empty', key: `${node.key} empty`, depth: depth + 1 })
      }
    })
  }
  walk(props.nodes, 0)
  return out
})

/**
 * The fonts of a label and of a hint. The view reads them from the first
 * drawn row that has each one, because the style sheet sets them.
 */
const labelFont = ref('')
const hintFont = ref('')
let meter: ReturnType<typeof createTextMeter> | null = null

function readFonts(): void {
  const label = scrollArea.value?.querySelector('.node-label')
  const hint = scrollArea.value?.querySelector('.node-hint')
  if (label) {
    labelFont.value = fontOf(label)
  }
  if (hint) {
    hintFont.value = fontOf(hint)
  }
}

onMounted(readFonts)
onUpdated(() => {
  if (labelFont.value === '' || hintFont.value === '') {
    readFonts()
  }
})

/**
 * The width of the widest row of the whole tree. The view draws the rows
 * near the visible part alone, so a width from the drawn rows would change
 * on each scroll. The width comes from a measure of each label and hint.
 */
const treeWidth = computed(() => {
  meter ??= createTextMeter(canvasContext())
  let widest = 0
  for (const row of rows.value) {
    if (row.rowType !== 'node') {
      continue
    }
    let width =
      row.depth * INDENT_STEP +
      ROW_PADDING +
      LABEL_OFFSET +
      meter(row.node.label, labelFont.value) +
      ROW_END
    if (row.node.hint) {
      width += HINT_GAP + meter(row.node.hint, hintFont.value)
    }
    widest = Math.max(widest, width)
  }
  return Math.ceil(widest)
})

/** The rows a key can reach, which leaves out the notes of empty and failed branches. */
const nodeRows = computed(() => rows.value.filter((row) => row.rowType === 'node'))

/** The first row of the window, counted from the first row of the tree. */
const firstDrawn = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - OVERSCAN))

/** The row after the last row of the window. */
const lastDrawn = computed(() =>
  Math.min(
    rows.value.length,
    firstDrawn.value + Math.ceil(viewportHeight.value / ROW_HEIGHT) + OVERSCAN * 2,
  ),
)

/** The rows the view draws. */
const windowRows = computed(() => rows.value.slice(firstDrawn.value, lastDrawn.value))

const topPad = computed(() => firstDrawn.value * ROW_HEIGHT)
const bottomPad = computed(() => (rows.value.length - lastDrawn.value) * ROW_HEIGHT)

function onScroll(event: Event): void {
  const target = event.target as HTMLElement
  scrollTop.value = target.scrollTop
  viewportHeight.value = target.clientHeight || viewportHeight.value
}

/**
 * Brings one row of the tree into the visible part. A row outside the window
 * is not drawn, so the place of the scroll moves before the focus goes to
 * the row.
 */
function scrollToRow(index: number): void {
  const area = scrollArea.value
  if (!area) {
    return
  }
  const top = index * ROW_HEIGHT
  const bottom = top + ROW_HEIGHT
  if (top < scrollTop.value) {
    area.scrollTop = top
  } else if (bottom > scrollTop.value + viewportHeight.value) {
    area.scrollTop = bottom - viewportHeight.value
  } else {
    return
  }
  // The place of the element and the place this view holds must agree at
  // once, because the window of the drawn rows follows this value and the
  // event of the scroll arrives later.
  scrollTop.value = area.scrollTop
}

/**
 * The row that carries the one tab stop of the tree. It is the row that holds
 * the focus, or the row the user chose, or else the first row. A tree with one
 * tab stop keeps a tree of five hundred nodes from holding five hundred of
 * them.
 */
const activeKey = computed(() => {
  const keys = nodeRows.value.map((row) => row.key)
  for (const candidate of [focusedKey.value, props.selectedKey]) {
    if (candidate !== null && keys.includes(candidate)) {
      return candidate
    }
  }
  return keys[0] ?? null
})

/** True when the row with the tab stop is among the drawn rows. */
const activeDrawn = computed(() => windowRows.value.some((row) => row.key === activeKey.value))

/**
 * Gives the focus to the row with the tab stop. The tree itself takes the
 * tab stop when that row is scrolled out of the drawn rows, so the Tab key
 * still reaches the tree.
 */
function onAreaFocus(): void {
  if (activeKey.value !== null) {
    focusRow(activeKey.value)
  }
}

function keepRow(key: string, element: Element | ComponentPublicInstance | null): void {
  if (element === null) {
    rowElements.delete(key)
    return
  }
  rowElements.set(key, element as HTMLElement)
}

function rowIndent(depth: number): string {
  return `${depth * INDENT_STEP + ROW_PADDING}px`
}

function labelIndent(depth: number): string {
  return `${depth * INDENT_STEP + ROW_PADDING + LABEL_OFFSET}px`
}

function activate(node: ExplorerNode): void {
  focusedKey.value = node.key
  emit('activate', node)
}

function openMenuAt(x: number, y: number, node: ExplorerNode): void {
  focusedKey.value = node.key
  emit('context', { x, y, node })
}

/**
 * Moves the focus to one row and brings that row into view. The browser
 * scrolls a focused element on both axes, which would lose the place of a
 * horizontal scroll on each step of a walk up or down, so the row asks for
 * the nearest edge of each axis instead.
 */
function focusRow(key: string): void {
  focusedKey.value = key
  scrollToRow(rows.value.findIndex((row) => row.key === key))
  void nextTick(() => {
    const row = rowElements.get(key)
    row?.focus({ preventScroll: true })
    row?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
}

/** The place of the row that holds the focus among the rows a key can reach. */
function activeIndex(): number {
  return nodeRows.value.findIndex((row) => row.key === activeKey.value)
}

function moveBy(step: number): void {
  const index = activeIndex()
  const next = nodeRows.value[index + step]
  if (next) {
    focusRow(next.key)
  }
}

function moveTo(index: number): void {
  const rowsToUse = nodeRows.value
  const row = rowsToUse[index < 0 ? rowsToUse.length + index : index]
  if (row) {
    focusRow(row.key)
  }
}

/**
 * Answers the Right key. A branch that is shut opens, and a branch that is
 * already open passes the focus to the first of its children.
 */
function onRight(row: Extract<Row, { rowType: 'node' }>): void {
  if (!row.expandable) {
    return
  }
  if (row.expanded) {
    moveBy(1)
    return
  }
  emit('expand', row.node)
}

/**
 * Answers the Left key. A branch that is open shuts, and any other row passes
 * the focus to the row that holds it.
 */
function onLeft(row: Extract<Row, { rowType: 'node' }>): void {
  if (row.expandable && row.expanded) {
    emit('collapse', row.node)
    return
  }
  const index = activeIndex()
  for (let above = index - 1; above >= 0; above -= 1) {
    const candidate = nodeRows.value[above]
    if (candidate && candidate.depth < row.depth) {
      focusRow(candidate.key)
      return
    }
  }
}

/**
 * Moves to the next row whose name begins with the letters just typed. Letters
 * that arrive close upon each other build one word, so `st` reaches Stock and
 * not Sales. One letter pressed again and again is the other case: it steps
 * through the names that begin with that letter, one for each press.
 */
function onType(letter: string): void {
  const now = Date.now()
  typed = now - typedAt > TYPE_AHEAD_MS ? letter : typed + letter
  typedAt = now

  const oneLetterAgain = [...typed].every((each) => each === typed[0])
  const wanted = oneLetterAgain ? typed.slice(0, 1) : typed
  // A search for one letter begins after the row that holds the focus, so the
  // same letter again reaches the next name of that letter. A search for a word
  // begins at the row itself, because the word grows on the row it found.
  const offset = oneLetterAgain ? 1 : 0

  const keys = nodeRows.value
  const start = activeIndex()
  for (let step = offset; step < keys.length + offset; step += 1) {
    const row = keys[(start + step) % keys.length]
    if (row && row.node.label.toLowerCase().startsWith(wanted)) {
      focusRow(row.key)
      return
    }
  }
}

function onKeyDown(event: KeyboardEvent): void {
  const row = nodeRows.value[activeIndex()]
  if (!row) {
    return
  }

  switch (event.key) {
    case 'ArrowDown':
      moveBy(1)
      break
    case 'ArrowUp':
      moveBy(-1)
      break
    case 'ArrowRight':
      onRight(row)
      break
    case 'ArrowLeft':
      onLeft(row)
      break
    case 'Home':
      moveTo(0)
      break
    case 'End':
      moveTo(-1)
      break
    // The standard name of the space bar is the space itself.
    case 'Enter':
    case ' ':
      activate(row.node)
      break
    case 'F10':
      // Shift and F10 is the key of the context menu on every host, and some
      // keyboards carry a key of their own for it.
      if (!event.shiftKey) {
        return
      }
      openMenuFor(row.key, row.node)
      break
    case 'ContextMenu':
      openMenuFor(row.key, row.node)
      break
    default:
      if (event.key.length !== 1 || event.altKey || event.ctrlKey || event.metaKey) {
        return
      }
      onType(event.key.toLowerCase())
      break
  }
  event.preventDefault()
}

/** Opens the menu of one row at the row itself, where the eye already is. */
function openMenuFor(key: string, node: ExplorerNode): void {
  const box = rowElements.get(key)?.getBoundingClientRect()
  openMenuAt(box ? box.left + 24 : 0, box ? box.bottom : 0, node)
}

defineExpose({ focusRow })
</script>

<style scoped>
/* The tree scrolls here, on both axes. */
.explorer-scroll {
  height: 100%;
  overflow: auto;
  outline: none;
}

/* Each row takes the width of the widest row, so a long name reaches past
   the panel and the scroll of the panel brings it into view. A tree that is
   narrower than the panel still fills the panel. The view sets a smallest
   width from the measure of every row, drawn or not. */
.explorer-tree {
  outline: none;
  width: max-content;
  min-width: max(100%, var(--tree-width));
}

/* Every row is as tall as every other one, because the window of the drawn
   rows counts the rows above it in one height. */
.tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
  height: var(--tree-row-height);
  box-sizing: border-box;
  padding-right: 8px;
  cursor: pointer;
  font-size: var(--app-text-md);
  user-select: none;
}

.tree-row:hover {
  background: rgba(var(--v-theme-primary), 0.08);
}

.tree-row:focus-visible {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}

.tree-row.selected {
  background: rgba(var(--v-theme-primary), 0.16);
}

/* An object that the engine keeps but does not run, such as a disabled
   trigger, shows in a paler text. */
.tree-row.dimmed .node-label,
.tree-row.dimmed .node-icon {
  opacity: 0.55;
}

.chevron,
.chevron-space {
  width: 16px;
  flex: 0 0 16px;
}

.node-icon {
  color: rgb(var(--v-theme-on-surface-variant));
}

.node-label {
  white-space: nowrap;
}

/* The hint sits against the right edge, so the hints of the rows align. */
.node-hint {
  margin-left: auto;
  padding-left: 8px;
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-on-surface-variant));
  white-space: nowrap;
}

.empty-branch {
  display: flex;
  align-items: center;
  height: var(--tree-row-height);
  box-sizing: border-box;
  font-size: var(--app-text-sm);
  font-style: italic;
  color: rgb(var(--v-theme-on-surface-variant));
}

.error-branch {
  gap: 4px;
  font-style: normal;
  color: rgb(var(--v-theme-error));
}

.error-text {
  white-space: nowrap;
}
</style>
