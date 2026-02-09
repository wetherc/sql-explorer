<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'

export interface MenuItem {
  label: string
  action: string
}

defineProps<{
  items: MenuItem[]
  x: number
  y: number
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'select', action: string): void
}>()

function handleClick(action: string) {
  emit('select', action)
  emit('close')
}

function handleOutsideClick(event: MouseEvent) {
  const target = event.target as HTMLElement
  if (!target.closest('.context-menu')) {
    emit('close')
  }
}

onMounted(() => {
  document.addEventListener('click', handleOutsideClick)
})

onUnmounted(() => {
  document.removeEventListener('click', handleOutsideClick)
})
</script>

<template>
  <div
    class="context-menu"
    :style="{ top: `${y}px`, left: `${x}px` }"
    @click.stop
  >
    <ul>
      <li v-for="item in items" :key="item.action" @click="handleClick(item.action)">
        {{ item.label }}
      </li>
    </ul>
  </div>
</template>

<style scoped>
.context-menu {
  position: absolute;
  background-color: white;
  border: 1px solid #ccc;
  box-shadow: 2px 2px 5px rgba(0, 0, 0, 0.1);
  min-width: 150px;
  z-index: 1000;
}

ul {
  list-style-type: none;
  padding: 5px 0;
  margin: 0;
}

li {
  padding: 8px 15px;
  cursor: pointer;
}

li:hover {
  background-color: #f0f0f0;
}
</style>
