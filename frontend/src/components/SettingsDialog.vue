<template>
  <AppDialog :model-value="open" size="medium" scrollable @update:model-value="close">
    <v-card>
      <v-card-title>Settings</v-card-title>
      <v-card-text class="d-flex flex-column ga-3">
        <!-- The settings are in groups, because fourteen controls in no
             order are hard to scan. -->
        <div class="settings-group">Appearance</div>
        <v-select
          :model-value="settings.settings.theme"
          :items="THEME_CHOICES"
          label="Theme"
          data-test="setting-theme"
          @update:model-value="(value) => settings.update({ theme: value })"
        />
        <v-slider
          :model-value="settings.settings.fontSize"
          label="Editor font size"
          :min="9"
          :max="24"
          :step="1"
          thumb-label
          hide-details
          data-test="setting-font-size"
          @update:model-value="(value) => settings.update({ fontSize: Number(value) })"
        />
        <v-switch
          :model-value="settings.settings.wordWrap"
          label="Wrap long lines"
          hide-details
          @update:model-value="(value) => settings.update({ wordWrap: Boolean(value) })"
        />
        <v-switch
          :model-value="settings.settings.showLineNumbers"
          label="Show line numbers"
          hide-details
          @update:model-value="(value) => settings.update({ showLineNumbers: Boolean(value) })"
        />

        <v-divider />
        <div class="settings-group">Results</div>
        <v-switch
          :model-value="settings.settings.autoRunPreview"
          label="Run table previews right away"
          hide-details
          @update:model-value="(value) => settings.update({ autoRunPreview: Boolean(value) })"
        />
        <NumberSetting
          :model-value="settings.settings.maxRows"
          label="Row limit"
          hint="Maximum rows fetched per result set, from 1 to 1,000,000."
          data-test="setting-max-rows"
          @update:model-value="(value) => settings.update({ maxRows: value })"
        />
        <NumberSetting
          :model-value="settings.settings.maxPinnedResults"
          label="Pinned results per tab"
          hint="How many pinned results each tab keeps across runs, from 1 to 20."
          data-test="setting-max-pinned"
          @update:model-value="(value) => settings.update({ maxPinnedResults: value })"
        />
        <NumberSetting
          :model-value="settings.settings.exportRowLimit"
          label="Export row limit"
          hint="Maximum rows for an export that streams straight to a file, from 1,000 to 100,000,000."
          data-test="setting-export-limit"
          @update:model-value="(value) => settings.update({ exportRowLimit: value })"
        />
        <v-switch
          :model-value="settings.settings.keepFullResults"
          color="primary"
          label="Save full results on this computer"
          hint="When a result passes the row limit, the query keeps reading up to the export row limit and saves every row, so Export all rows doesn't run it again. Runs take longer."
          persistent-hint
          data-test="setting-keep-full-results"
          @update:model-value="(value) => settings.update({ keepFullResults: value === true })"
        />
        <NumberSetting
          :model-value="settings.settings.fullResultsDiskGb"
          label="Disk space for saved results (gigabytes)"
          hint="From 1 to 1,024. When a new result needs more space, the oldest saved result is deleted first."
          :disabled="!settings.settings.keepFullResults"
          data-test="setting-full-results-disk"
          @update:model-value="(value) => settings.update({ fullResultsDiskGb: value })"
        />
        <v-switch
          :model-value="settings.settings.pauseAtRowLimit"
          color="primary"
          label="Pause queries at the row limit"
          hint="When a result passes the row limit, the query pauses with its statement still open on the server, so Export all rows continues it without running it again. Only a single read statement in a tab pauses. While paused, the tab can't run anything else, and the server can keep locks on the rows it read. Not used while full results are saved on this computer."
          persistent-hint
          data-test="setting-pause-at-row-limit"
          @update:model-value="(value) => settings.update({ pauseAtRowLimit: value === true })"
        />
        <NumberSetting
          :model-value="settings.settings.pauseLimitMinutes"
          label="Pause time limit (minutes)"
          hint="From 1 to 60. A paused query that isn't exported by then is released."
          :disabled="!settings.settings.pauseAtRowLimit"
          data-test="setting-pause-limit"
          @update:model-value="(value) => settings.update({ pauseLimitMinutes: value })"
        />

        <v-divider />
        <div class="settings-group">Athena</div>
        <NumberSetting
          :model-value="settings.settings.athenaPricePerTerabyte"
          label="Athena price per terabyte"
          step="0.01"
          prefix="$"
          hint="Used for cost estimates. The actual rate depends on your region and contract."
          data-test="setting-athena-price"
          @update:model-value="(value) => settings.update({ athenaPricePerTerabyte: value })"
        />
        <NumberSetting
          :model-value="settings.settings.athenaScanWarningGb"
          label="Scan warning threshold (gigabytes)"
          hint="Warn when a statement scans more than this, from 1 to 1,000,000."
          data-test="setting-athena-warning"
          @update:model-value="(value) => settings.update({ athenaScanWarningGb: value })"
        />

        <v-divider />
        <div class="settings-group">Autocomplete</div>
        <NumberSetting
          :model-value="settings.settings.schemaSnapshotColumns"
          label="Column limit"
          hint="Maximum columns loaded per schema for autocomplete, from 100 to 200,000."
          data-test="setting-snapshot-columns"
          @update:model-value="(value) => settings.update({ schemaSnapshotColumns: value })"
        />
        <v-switch
          :model-value="settings.settings.schemaSnapshotOwnConnection"
          color="primary"
          label="Load schema on a separate connection"
          hint="Uses one more server session, but never waits for your running statements."
          persistent-hint
          data-test="setting-snapshot-connection"
          @update:model-value="
            (value) => settings.update({ schemaSnapshotOwnConnection: value === true })
          "
        />
      </v-card-text>
      <v-card-actions>
        <v-btn
          text="Reset to defaults"
          data-test="settings-reset"
          @click="resettingSettings = true"
        />
        <v-spacer />
        <v-btn text="Close" @click="close" />
      </v-card-actions>
    </v-card>
  </AppDialog>

  <ConfirmDialog
    :open="resettingSettings"
    title="Reset all settings?"
    message="Every setting goes back to its default value."
    confirm-text="Reset"
    danger
    @confirm="resetSettings"
    @cancel="resettingSettings = false"
  />
</template>

<script setup lang="ts">
import { ref } from 'vue'
import AppDialog from './AppDialog.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import NumberSetting from './NumberSetting.vue'
import { THEME_CHOICES, useSettingsStore } from '@/stores/settings'

defineProps<{ open: boolean }>()
const emit = defineEmits<{ (event: 'update:open', value: boolean): void }>()

const settings = useSettingsStore()

/** True while the question about resetting the settings is open. */
const resettingSettings = ref(false)

function close(): void {
  emit('update:open', false)
}

function resetSettings(): void {
  resettingSettings.value = false
  settings.reset()
}
</script>

<style scoped>
/* The name of one group of settings, above the controls that belong to it. */
.settings-group {
  font-size: var(--app-text-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: rgb(var(--v-theme-on-surface-variant));
}
</style>
