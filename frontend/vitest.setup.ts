import { vi } from 'vitest';
import { createApp } from 'vue';
import PrimeVue from 'primevue/config';
import Aura from '@primeuix/themes/aura';

// Mock monaco-editor-vue3
vi.mock('monaco-editor-vue3', () => ({
  default: {
    template: `<textarea :value="modelValue" @input="$emit('update:modelValue', ($event.target).value)"></textarea>`,
    props: ['modelValue', 'theme', 'lang', 'options'],
    emits: ['update:modelValue'],
  },
}));

// Global PrimeVue setup for tests
const app = createApp({});
app.use(PrimeVue, {
  unstyled: true,
  theme: {
    preset: Aura
  }
});

// Mock window.matchMedia which is used by PrimeVue
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

