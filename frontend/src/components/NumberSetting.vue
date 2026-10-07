<template>
  <v-text-field
    v-model="text"
    :label="label"
    type="number"
    :step="step"
    :prefix="prefix"
    :hint="hint"
    persistent-hint
    @update:focused="(focused: boolean) => !focused && commit()"
    @keydown.enter="commit"
  />
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'

/**
 * A number box of the settings. It sends its value when the user leaves the
 * box or presses Enter, and not on each keystroke. The store limits each
 * setting to a range, so a value sent on each keystroke would turn the "1"
 * of a typed "1500" into the lowest value of the range at once.
 */
const props = defineProps<{
  modelValue: number
  label: string
  hint?: string
  prefix?: string
  step?: string | number
}>()
const emit = defineEmits<{ (event: 'update:modelValue', value: number): void }>()

const text = ref(String(props.modelValue))

// The store can change the value, by a limit or by a reset, so the box
// shows the value the store keeps.
watch(
  () => props.modelValue,
  (value) => {
    text.value = String(value)
  },
)

function commit(): void {
  const value = Number(text.value)
  if (text.value.trim() === '' || !Number.isFinite(value)) {
    text.value = String(props.modelValue)
    return
  }
  emit('update:modelValue', value)
  // A value that the store limits to the same number as before sends no
  // change back, so the box takes the value of the store here.
  text.value = String(props.modelValue)
}
</script>
