import { vi } from 'vitest';

vi.mock('monaco-editor-vue3', () => ({
  default: {
    template: `<textarea :value="modelValue" @input="$emit('update:modelValue', ($event.target).value)"></textarea>`,

    props: ['modelValue', 'theme', 'lang', 'options'],
    emits: ['update:modelValue'],
  },
}));
