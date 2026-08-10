<template>
  <div class="explorer-tree" role="tree" aria-label="Database objects" @keydown="onKeyDown">
    <template v-for="row in rows" :key="row.key">
      <div
        v-if="row.kind === 'node'"
        :ref="(element) => keepRow(row.key, element)"
        class="tree-row"
        :class="{ selected: selectedKey === row.key }"
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

        <!-- A long name is cut short on screen, so the whole name waits under
             the pointer. -->
        <span class="node-label" :title="row.node.label">{{ row.node.label }}</span>
        <span v-if="row.node.hint" class="node-hint">{{ row.node.hint }}</span>
      </div>

      <div v-else class="empty-branch" :style="{ paddingLeft: labelIndent(row.depth) }">
        Nothing here
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, type ComponentPublicInstance } from 'vue'
import { isExpandable, type ExplorerNode } from '@/stores/explorer'

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

/** How long a type-ahead holds its letters before it starts again. */
const TYPE_AHEAD_MS = 800

const props = withDefaults(
  defineProps<{
    nodes: ExplorerNode[]
    openKeys: Set<string>
    selectedKey?: string | null
  }>(),
  { selectedKey: null },
)

const emit = defineEmits<{
  (event: 'activate', node: ExplorerNode): void
  (event: 'expand', node: ExplorerNode): void
  (event: 'collapse', node: ExplorerNode): void
  (event: 'context', payload: { x: number; y: number; node: ExplorerNode }): void
}>()

/** One line of the tree: either a node, or the note of a branch with none. */
type Row =
  | {
      kind: 'node'
      key: string
      node: ExplorerNode
      depth: number
      posInSet: number
      setSize: number
      expandable: boolean
      expanded: boolean
    }
  | { kind: 'empty'; key: string; depth: number }

/** The row that holds the focus, which is the one row the Tab key reaches. */
const focusedKey = ref<string | null>(null)
const rowElements = new Map<string, HTMLElement>()

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
        kind: 'node',
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
      if (children.length > 0) {
        walk(children, depth + 1)
      } else if (node.loaded) {
        out.push({ kind: 'empty', key: `${node.key} empty`, depth: depth + 1 })
      }
    })
  }
  walk(props.nodes, 0)
  return out
})

/** The rows a key can reach, which leaves out the note of an empty branch. */
const nodeRows = computed(() => rows.value.filter((row) => row.kind === 'node'))

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

/** Moves the focus to one row and lets the browser draw it there. */
function focusRow(key: string): void {
  focusedKey.value = key
  void nextTick(() => rowElements.get(key)?.focus())
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
function onRight(row: Extract<Row, { kind: 'node' }>): void {
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
function onLeft(row: Extract<Row, { kind: 'node' }>): void {
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
.explorer-tree {
  outline: none;
}

.tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
  padding-top: 3px;
  padding-bottom: 3px;
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

.chevron,
.chevron-space {
  width: 16px;
  flex: 0 0 16px;
}

.node-icon {
  color: rgb(var(--v-theme-on-surface-variant));
}

.node-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-hint {
  margin-left: 8px;
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-on-surface-variant));
  white-space: nowrap;
}

.empty-branch {
  font-size: var(--app-text-sm);
  font-style: italic;
  color: rgb(var(--v-theme-on-surface-variant));
  padding-top: 2px;
  padding-bottom: 2px;
}
</style>
