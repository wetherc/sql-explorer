<template>
  <AppDialog :model-value="open" size="medium" @update:model-value="cancel">
    <v-card>
      <v-card-title>Run to file</v-card-title>
      <v-card-text>
        <p class="path" data-test="run-file-path">Saving to {{ path }}</p>
        <v-alert
          v-if="excelOver"
          type="warning"
          density="compact"
          variant="tonal"
          class="mt-3"
          data-test="run-file-excel-limit"
        >
          An Excel sheet has room for {{ SHEET_ROW_ROOM.toLocaleString() }} rows, and your export
          row limit is {{ rowLimit.toLocaleString() }}. Rows past the end of a sheet won't be saved.
          <template #append>
            <v-btn
              variant="text"
              size="small"
              text="Save as CSV instead…"
              data-test="run-file-csv"
              @click="emit('csv')"
            />
          </template>
        </v-alert>
        <template v-if="several">
          <p class="mt-3">This script can return more than one result.</p>
          <v-radio-group v-model="choice" hide-details density="compact" data-test="run-file-sets">
            <v-radio label="First result only" value="first" data-test="run-file-first" />
            <v-radio :label="eachSetLabel(format)" value="each" data-test="run-file-each" />
          </v-radio-group>
          <p
            v-if="choice === 'each' && format !== 'xlsx'"
            class="app-text-sm text-medium-emphasis mt-2"
            data-test="run-file-names"
          >
            The second result goes to {{ secondFileName(path) }} beside it, and so on. A name that's
            taken gets a number, so no file is replaced.
          </p>
        </template>
      </v-card-text>
      <v-card-actions>
        <v-spacer />
        <v-btn text="Cancel" data-test="run-file-cancel" @click="cancel" />
        <v-btn color="primary" variant="flat" text="Run" data-test="run-file-run" @click="run" />
      </v-card-actions>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import AppDialog from './AppDialog.vue'
import {
  SHEET_ROW_ROOM,
  eachSetLabel,
  excelRowsOver,
  loadSetChoice,
  saveSetChoice,
  secondFileName,
  type SetChoice,
} from '@/lib/runFile'
import type { RunFileFormat } from '@/types/api'

/**
 * The question that comes after the save dialog of a run to a file, when the
 * script can return more than one result or when the export row limit is
 * past the room of an Excel sheet. The choice of the last run is selected
 * when the dialog opens.
 */
const props = defineProps<{
  open: boolean
  path: string
  format: RunFileFormat
  /** True when the script can return more than one result. */
  several: boolean
  /** The export row limit of each result. */
  rowLimit: number
}>()

const emit = defineEmits<{
  /** The user chose to run. The flag is true when each result goes to the
   *  file. */
  (event: 'run', eachSet: boolean): void
  (event: 'cancel'): void
  /** The user asked to choose a CSV file in place of the Excel file. */
  (event: 'csv'): void
}>()

const excelOver = computed(() => excelRowsOver(props.format, props.rowLimit))

const choice = ref<SetChoice>(loadSetChoice())

watch(
  () => props.open,
  (open) => {
    if (open) {
      choice.value = loadSetChoice()
    }
  },
)

function run(): void {
  if (props.several) {
    saveSetChoice(choice.value)
  }
  emit('run', props.several && choice.value === 'each')
}

function cancel(): void {
  emit('cancel')
}
</script>

<style scoped>
.path {
  overflow-wrap: anywhere;
}
</style>
