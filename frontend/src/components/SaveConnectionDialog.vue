<script setup lang="ts">
import { ref } from 'vue'
import type { SavedConnection } from '@/types/savedConnection'

const props = defineProps<{
  show: boolean
  connection: Omit<SavedConnection, 'name'>
  password?: string
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'save', name: string, password?: string): void
}>()

const connectionName = ref('')
const savePassword = ref(false)

function onSave() {
  if (connectionName.value.trim()) {
    emit('save', connectionName.value.trim(), savePassword.value ? props.password : undefined)
    onClose()
  } else {
    alert('Please enter a name for the connection.')
  }
}

function onClose() {
  connectionName.value = ''
  savePassword.value = false
  emit('close')
}
</script>

<template>
  <div v-if="show" class="modal-overlay" @click.self="onClose">
    <div class="modal-content">
      <h3>Save Connection</h3>
      <form @submit.prevent="onSave">
        <div class="form-group">
          <label for="connection-name">Connection Name</label>
          <input
            id="connection-name"
            v-model="connectionName"
            type="text"
            required
            placeholder="e.g., My Local DB"
          />
        </div>
        <div v-if="connection.auth_type === 'Sql'" class="form-group">
          <label>
            <input type="checkbox" v-model="savePassword" />
            Save Password (insecure, for local development only)
          </label>
        </div>
        <div class="form-actions">
          <button type="button" @click="onClose">Cancel</button>
          <button type="submit">Save</button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background-color: rgba(0, 0, 0, 0.5);
  display: flex;
  justify-content: center;
  align-items: center;
}
.modal-content {
  background: white;
  padding: 20px;
  border-radius: 8px;
  min-width: 300px;
}
.form-group {
  margin-bottom: 15px;
}
label {
  display: block;
  margin-bottom: 5px;
}
input[type='text'] {
  width: 100%;
  padding: 8px;
  border: 1px solid #ccc;
  border-radius: 4px;
}
.form-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
