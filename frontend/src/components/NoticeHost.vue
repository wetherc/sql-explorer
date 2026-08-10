<template>
  <div class="notice-host">
    <!-- Each open notice moves up by its place in the list, so two notices
         at the same corner do not cover each other. -->
    <v-snackbar
      v-for="(notice, index) in ui.notices"
      :key="notice.id"
      :model-value="true"
      :color="notice.level"
      :timeout="notice.timeout"
      location="bottom right"
      :style="{ marginBottom: `${index * 64}px` }"
      data-test="notice"
      @update:model-value="ui.dismiss(notice.id)"
    >
      <!-- A notice arrives on its own, so a reader is told of it as it comes.
           An error breaks in, because it stops the work of the user. The
           snackbar is drawn away from this element, so the part that a reader
           follows is the text of the notice itself. -->
      <div
        class="d-flex align-center ga-2"
        :role="notice.level === 'error' ? 'alert' : 'status'"
        :aria-live="notice.level === 'error' ? 'assertive' : 'polite'"
      >
        <v-icon size="small" aria-hidden="true">{{ notice.icon }}</v-icon>
        <span class="notice-text">{{ notice.message }}</span>
      </div>
      <template #actions>
        <v-btn
          v-if="notice.detail"
          size="small"
          text="Details"
          data-test="notice-details"
          @click="ui.openNotice(notice)"
        />
        <v-btn
          icon="mdi-close"
          size="x-small"
          aria-label="Dismiss"
          data-test="notice-close"
          @click="ui.dismiss(notice.id)"
        />
      </template>
    </v-snackbar>

    <AppDialog
      :model-value="ui.openedNotice !== null"
      max-width="720"
      @update:model-value="ui.closeNotice()"
    >
      <v-card v-if="ui.openedNotice">
        <v-card-title class="text-subtitle-1 d-flex align-center ga-2">
          <v-icon :color="ui.openedNotice.level">{{ ui.openedNotice.icon }}</v-icon>
          {{ ui.openedNotice.message }}
        </v-card-title>
        <v-card-text>
          <pre class="app-code-block" data-test="notice-detail-body">{{
            ui.openedNotice.detail
          }}</pre>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Close" @click="ui.closeNotice()" />
        </v-card-actions>
      </v-card>
    </AppDialog>
  </div>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
</script>

<style scoped>
.notice-text {
  max-width: 460px;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
