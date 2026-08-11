<template>
  <AppDialog :model-value="open" max-width="900" scrollable @update:model-value="close">
    <v-card class="guide-card">
      <v-card-title class="text-subtitle-1">Guide</v-card-title>
      <v-card-text class="guide-body d-flex ga-4">
        <!-- The list of topics stands beside the topic, because the reader
             moves between the topics while they read. -->
        <v-list density="compact" nav class="topic-list" aria-label="Topics">
          <v-list-item
            v-for="topic of topics"
            :key="topic.id"
            :active="topic.id === activeId"
            :title="topic.title"
            :data-test="`guide-topic-${topic.id}`"
            @click="activeId = topic.id"
          />
        </v-list>

        <div class="topic-body" data-test="guide-content">
          <h2 class="text-subtitle-1 mb-2">{{ activeTopic.title }}</h2>
          <p>{{ activeTopic.body }}</p>
        </div>
      </v-card-text>
      <v-card-actions>
        <v-spacer />
        <v-btn text="Close" data-test="guide-close" @click="close(false)" />
      </v-card-actions>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import AppDialog from './AppDialog.vue'
import { GUIDE_TOPICS, topicById } from '@/lib/guide'

defineProps<{ open: boolean }>()
const emit = defineEmits<{ (event: 'update:open', open: boolean): void }>()

const topics = GUIDE_TOPICS
const activeId = ref(topics[0]!.id)

const activeTopic = computed(() => topicById(activeId.value))

function close(open: boolean): void {
  emit('update:open', open)
}
</script>

<style scoped>
.guide-body {
  min-height: 320px;
}

.topic-list {
  flex: 0 0 200px;
  border-right: var(--app-divider-soft);
}

.topic-body {
  flex: 1 1 auto;
  min-width: 0;
  overflow: auto;
}
</style>
