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
          <!-- The text comes from a file of the build and holds no text of
               the user, so it needs no cleaning step. -->
          <!-- eslint-disable-next-line vue/no-v-html -->
          <div class="topic-text" v-html="topicHtml"></div>
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
import { GUIDE_TOPICS, renderTopic, topicById } from '@/lib/guide'

defineProps<{ open: boolean }>()
const emit = defineEmits<{ (event: 'update:open', open: boolean): void }>()

const topics = GUIDE_TOPICS
const activeId = ref(topics[0]!.id)

const activeTopic = computed(() => topicById(activeId.value))
const topicHtml = computed(() => renderTopic(activeTopic.value))

function close(open: boolean): void {
  emit('update:open', open)
}
</script>

<style scoped>
/* The dialog holds one size, whatever the topic that stands open, so the
   window does not jump as the reader moves from topic to topic. The text of
   a long topic scrolls inside it. */
.guide-card {
  height: 70vh;
}

.guide-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
}

.topic-list {
  flex: 0 0 200px;
  overflow-y: auto;
  border-right: var(--app-divider-soft);
}

.topic-body {
  flex: 1 1 auto;
  min-width: 0;
  overflow: auto;
}

/* The rules below reach the HTML that the renderer made, which carries no
   class of the component, so they name the elements themselves. Each colour
   comes from the theme, so the text reads under both themes. */
.topic-text {
  font-size: var(--app-text-md);
  line-height: 1.6;
}

.topic-text :deep(h2),
.topic-text :deep(h3) {
  margin: 16px 0 6px;
  font-size: var(--app-text-lg);
  font-weight: 600;
}

.topic-text :deep(p),
.topic-text :deep(ul),
.topic-text :deep(ol) {
  margin: 0 0 10px;
}

.topic-text :deep(ul),
.topic-text :deep(ol) {
  padding-left: 20px;
}

.topic-text :deep(li) {
  margin: 2px 0;
}

.topic-text :deep(code) {
  padding: 1px 4px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.1);
  font-family: var(--app-font-mono);
  font-size: var(--app-text-sm);
}

.topic-text :deep(pre) {
  margin: 0 0 10px;
  padding: 8px;
  overflow-x: auto;
  border-radius: 4px;
  background: rgba(var(--v-theme-on-surface), 0.07);
}

.topic-text :deep(pre code) {
  padding: 0;
  background: none;
}

.topic-text :deep(a) {
  color: rgb(var(--v-theme-primary));
}

.topic-text :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin: 0 0 10px;
}

.topic-text :deep(th),
.topic-text :deep(td) {
  padding: 4px 8px;
  text-align: left;
  border-bottom: var(--app-divider-soft);
}
</style>
