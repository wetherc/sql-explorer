<template>
  <v-dialog v-bind="$attrs" :model-value="modelValue" @update:model-value="onChange">
    <slot />
  </v-dialog>
</template>

<script setup lang="ts">
import { nextTick, onBeforeUnmount, watch } from 'vue'
import { useUiStore } from '@/stores/ui'

/**
 * A dialog that counts itself in the store while it stands open.
 *
 * The shell listens for the keys of the application on the window, and a key
 * pressed inside a dialog reaches it there. The shell therefore has to know
 * that a dialog stands open, and it asked the document before. A count in the
 * store says the same thing without a search of the document, and it holds
 * whatever the component library names its classes.
 */
// The attributes reach the dialog below through `v-bind`, so they must not
// also land on it as the attributes of this component.
defineOptions({ inheritAttrs: false })

const props = defineProps<{ modelValue: boolean }>()
const emit = defineEmits<{ (event: 'update:modelValue', value: boolean): void }>()

const ui = useUiStore()
let counted = false
/** The element that held the focus before the dialog took it. */
let opener: HTMLElement | null = null

function count(open: boolean): void {
  if (open === counted) {
    return
  }
  counted = open
  if (open) {
    ui.addDialog()
    rememberOpener()
  } else {
    ui.removeDialog()
    returnFocus()
  }
}

/**
 * Holds the element the focus stood on. A dialog takes the focus from
 * whatever opened it, and the focus has to go back there when the dialog
 * closes, or the user of a keyboard is left at the top of the document.
 */
function rememberOpener(): void {
  const active = document.activeElement
  opener = active instanceof HTMLElement && active !== document.body ? active : null
}

function returnFocus(): void {
  const target = opener
  opener = null
  if (!target || !target.isConnected) {
    return
  }
  // The dialog is still going away, so the focus waits for it to finish.
  void nextTick(() => target.focus())
}

function onChange(value: boolean): void {
  emit('update:modelValue', value)
}

watch(() => props.modelValue, count, { immediate: true })

// A dialog that goes away while it stands open must still take itself off the
// count, or the shell would hold its keys for ever.
onBeforeUnmount(() => count(false))
</script>
