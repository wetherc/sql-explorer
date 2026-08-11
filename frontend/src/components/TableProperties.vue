<template>
  <AppDialog :model-value="open" max-width="820" @update:model-value="emit('close')">
    <v-card data-test="properties-dialog">
      <v-card-title class="text-subtitle-1">{{ title }}</v-card-title>

      <v-card-text>
        <v-progress-linear v-if="loading" indeterminate class="mb-3" />

        <!-- The read of the properties can fail on its own, so the dialog
             holds the failure and offers the read again. Closing the dialog
             and pushing a notice would take the user away from what they
             asked for. -->
        <v-alert
          v-else-if="failure"
          type="error"
          variant="tonal"
          density="compact"
          data-test="properties-error"
        >
          <div class="font-weight-medium">{{ failure.message }}</div>
          <pre v-if="failure.detail" class="app-code-block mt-2">{{ failure.detail }}</pre>
        </v-alert>

        <template v-else-if="details">
          <div class="text-subtitle-2 mb-1">General</div>
          <v-table density="compact" class="mb-4">
            <tbody>
              <tr v-for="fact of details.facts" :key="fact.name" data-test="property-fact">
                <td class="text-medium-emphasis">{{ fact.name }}</td>
                <td>{{ fact.value }}</td>
              </tr>
              <tr v-if="details.facts.length === 0">
                <td class="text-medium-emphasis" data-test="no-facts">
                  This engine reports no facts for a relation.
                </td>
              </tr>
            </tbody>
          </v-table>

          <div class="text-subtitle-2 mb-1">Columns</div>
          <v-table density="compact" class="mb-4">
            <thead>
              <tr>
                <th>Name</th>
                <th>Type</th>
                <th>Empty values</th>
                <th>Key</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="column of details.columns" :key="column.name" data-test="property-column">
                <td>{{ column.name }}</td>
                <td>{{ column.dataType }}</td>
                <td>{{ column.nullable ? 'yes' : 'no' }}</td>
                <td>{{ column.isPrimaryKey ? 'yes' : '' }}</td>
              </tr>
            </tbody>
          </v-table>

          <div class="text-subtitle-2 mb-1">Indexes</div>
          <v-table density="compact" class="mb-4">
            <tbody>
              <tr v-for="index of details.indexes" :key="index.name" data-test="property-index">
                <td>{{ index.name }}</td>
                <td>{{ index.columns.join(', ') }}</td>
                <td class="text-medium-emphasis">{{ indexRule(index) }}</td>
              </tr>
              <tr v-if="details.indexes.length === 0">
                <td class="text-medium-emphasis" data-test="no-indexes">
                  This relation holds no index.
                </td>
              </tr>
            </tbody>
          </v-table>

          <div class="text-subtitle-2 mb-1">Keys</div>
          <v-table density="compact">
            <tbody>
              <tr
                v-for="constraint of details.constraints"
                :key="constraint.name"
                data-test="property-constraint"
              >
                <td>{{ constraint.name }}</td>
                <td class="text-medium-emphasis">{{ constraintHint(constraint) }}</td>
              </tr>
              <tr v-if="details.constraints.length === 0">
                <td class="text-medium-emphasis" data-test="no-constraints">
                  This relation holds no key.
                </td>
              </tr>
            </tbody>
          </v-table>
        </template>
      </v-card-text>

      <v-card-actions>
        <v-spacer />
        <v-btn
          v-if="failure && node"
          color="primary"
          variant="flat"
          text="Try again"
          data-test="properties-retry"
          @click="read(node)"
        />
        <v-btn text="Close" data-test="properties-close" @click="emit('close')" />
      </v-card-actions>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { computed, ref, watch } from 'vue'
import { api } from '@/lib/api'
import { toErrorPayload } from '@/lib/errors'
import { constraintHint, type ExplorerNode } from '@/stores/explorer'
import type { ErrorPayload, IndexRef, TableDetails } from '@/types/api'

const props = defineProps<{ open: boolean; node: ExplorerNode | null }>()
const emit = defineEmits<{ (event: 'close'): void }>()

const details = ref<TableDetails | null>(null)
const loading = ref(false)
/** What the read of the properties reported, while it failed. */
const failure = ref<ErrorPayload | null>(null)

const title = computed(() => {
  const node = props.node
  if (!node) {
    return 'Properties'
  }
  const place = [node.database, node.schema].filter(Boolean).join('.')
  return place ? `${place}.${node.label}` : node.label
})

/** Names what an index does, beside the columns it covers. */
function indexRule(index: IndexRef): string {
  if (index.primary) {
    return 'primary key'
  }
  return index.unique ? 'unique' : ''
}

async function read(node: ExplorerNode): Promise<void> {
  loading.value = true
  details.value = null
  failure.value = null
  try {
    details.value = await api.tableDetails(
      node.connectionId,
      node.database ?? '',
      node.schema ?? null,
      node.table ?? node.label,
    )
  } catch (error) {
    failure.value = toErrorPayload(error)
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.open, props.node?.key],
  () => {
    if (props.open && props.node) {
      void read(props.node)
    }
  },
  { immediate: true },
)
</script>
