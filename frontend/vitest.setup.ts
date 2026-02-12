import { vi } from 'vitest';
import { createApp } from 'vue';
import vuetify from './src/plugins/vuetify'; // Correctly import the configured Vuetify instance

// Mock monaco-editor-vue3 if used in tests
vi.mock('monaco-editor-vue3', () => ({
  default: {
    template: `<textarea :value="modelValue" @input="$emit('update:modelValue', ($event.target as HTMLTextAreaElement).value)"></textarea>`,
    props: ['modelValue', 'theme', 'lang', 'options'],
    emits: ['update:modelValue'],
  },
}));

// Global Vuetify setup for tests
const app = createApp({});
app.use(vuetify);

// Mock window.matchMedia which is used by Vuetify
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation(query => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(), // deprecated
    removeListener: vi.fn(), // deprecated
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});
