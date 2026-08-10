<template>
  <AppDialog :model-value="open" max-width="460" @update:model-value="cancel">
    <v-card>
      <v-card-title class="text-subtitle-1">{{ title }}</v-card-title>
      <v-card-text>
        <slot>{{ message }}</slot>
      </v-card-text>
      <v-card-actions>
        <v-spacer />
        <v-btn :text="cancelText" data-test="confirm-cancel" @click="cancel" />
        <!-- The button that acts carries the colour of what it does and stands
             out from the one beside it, so the two do not look alike. -->
        <v-btn
          :color="danger ? 'error' : 'primary'"
          variant="flat"
          :text="confirmText"
          data-test="confirm-accept"
          @click="accept"
        />
      </v-card-actions>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'

/**
 * The question the application asks before it does something that the user
 * cannot undo. Every such question uses this component, so all of them hold
 * the same shape and the same order of buttons.
 */
withDefaults(
  defineProps<{
    open: boolean
    title: string
    /** The body of the question. A slot takes its place when one is given. */
    message?: string
    confirmText?: string
    cancelText?: string
    /** True when the action takes something away. */
    danger?: boolean
  }>(),
  { message: '', confirmText: 'Yes', cancelText: 'Cancel', danger: false },
)

const emit = defineEmits<{ (event: 'confirm'): void; (event: 'cancel'): void }>()

function accept(): void {
  emit('confirm')
}

function cancel(): void {
  emit('cancel')
}
</script>
