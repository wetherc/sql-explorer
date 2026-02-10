<script setup lang="ts">
import { ref, watch } from 'vue'
import { useToast } from 'primevue/usetoast'
import type { SavedConnection } from '@/types/savedConnection'
import { AuthType } from '@/types/savedConnection'

import Button from 'primevue/button'
import Checkbox from 'primevue/checkbox'
import Dialog from 'primevue/dialog'
import FloatLabel from 'primevue/floatlabel'
import InputText from 'primevue/inputtext'


const props = defineProps<{
  visible: boolean
  connection: Omit<SavedConnection, 'name'>
  password?: string
}>()

const emit = defineEmits<{
  (e: 'update:visible', value: boolean): void
  (e: 'save', name: string, password?: string): void
}>()

const toast = useToast()
const connectionName = ref('')
const savePassword = ref(false)

watch(() => props.visible, (newValue) => {
  if (!newValue) {
    // Reset local state when dialog is closed
    connectionName.value = ''
    savePassword.value = false
  }
})

function onSave() {
  if (connectionName.value.trim()) {
    emit('save', connectionName.value.trim(), savePassword.value ? props.password : undefined)
    closeDialog()
  } else {
    toast.add({ severity: 'warn', summary: 'Validation Error', detail: 'Please enter a name for the connection.', life: 3000 });
  }
}

function closeDialog() {
  emit('update:visible', false)
}
</script>

<template>
  <Dialog
    :visible="visible"
    modal
    header="Save Connection"
    :style="{ width: '25rem' }"
    @update:visible="closeDialog"
  >
    <form @submit.prevent="onSave" class="p-fluid">
      <div class="field mt-4">
        <FloatLabel>
          <InputText
            id="connection-name"
            v-model="connectionName"
            required
            autofocus
          />
          <label for="connection-name">Connection Name</label>
        </FloatLabel>
      </div>
      <div v-if="connection.auth_type === AuthType.Sql" class="field-checkbox">
        <Checkbox v-model="savePassword" input-id="savePassword" :binary="true" />
        <label for="savePassword" class="ml-2"> Save Password (insecure) </label>
      </div>
      <div class="form-actions flex justify-content-end gap-2 mt-4">
        <Button type="button" label="Cancel" severity="secondary" @click="closeDialog" />
        <Button type="submit" label="Save" />
      </div>
    </form>
  </Dialog>
</template>

<style scoped>
/* Scoped styles can be removed if not needed, as PrimeVue handles most styling */
</style>
