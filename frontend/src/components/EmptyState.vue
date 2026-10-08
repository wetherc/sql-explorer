<template>
  <div class="empty-state" :class="`empty-state--${size}`">
    <v-icon :size="iconSize" class="empty-state-icon mb-3">{{ icon }}</v-icon>
    <div class="empty-state-title mb-1">{{ title }}</div>
    <p v-if="hint" class="empty-state-hint text-medium-emphasis mb-4">
      {{ hint }}
    </p>
    <!-- The action of the state, when one exists. A state without an action
         leaves the slot empty and keeps its text alone. -->
    <slot />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

/**
 * The text and the icon that stand in the place of content the user does not
 * have yet. Every empty place in the application uses this component, so all
 * of them hold the same shape.
 *
 * A `panel` state fits one of the side panels. A `page` state fills the whole
 * work area.
 */
const props = withDefaults(
  defineProps<{
    icon: string
    title: string
    hint?: string
    size?: 'panel' | 'page'
  }>(),
  { hint: undefined, size: 'panel' },
)

const iconSize = computed(() => (props.size === 'page' ? 56 : 40))
</script>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
}

/* The icon stays quiet, because the action below it holds the colour that
   asks for a click. */
.empty-state-icon {
  color: rgb(var(--v-theme-on-surface-variant));
}

.empty-state-title {
  font-weight: 500;
}

.empty-state-hint {
  max-width: 34ch;
}

.empty-state--panel {
  padding: 24px 16px;
}

.empty-state--panel .empty-state-title {
  font-size: var(--app-text-md);
}

.empty-state--panel .empty-state-hint {
  font-size: var(--app-text-sm);
}

.empty-state--page {
  height: 100%;
  padding: 24px;
}

.empty-state--page .empty-state-title {
  font-size: var(--app-text-lg);
}

.empty-state--page .empty-state-hint {
  font-size: var(--app-text-md);
}
</style>
