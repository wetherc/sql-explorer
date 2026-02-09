import { describe, it, expect, vi, beforeEach } from 'vitest';
import { shallowMount } from '@vue/test-utils';
import QueryView from '../QueryView.vue';
import MonacoEditor from 'monaco-editor-vue3';

// Mock MonacoEditor
vi.mock('monaco-editor-vue3', () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  default: {
    name: 'MonacoEditor',
    template: '<textarea :value="modelValue" @input="$emit('update:modelValue', ($event.target as HTMLTextAreaElement).value)"></textarea>',
    props: ['modelValue'],
    emits: ['update:modelValue'],
  },
}));

describe('QueryView.vue', () => {
  const defaultProps = {
    query: 'SELECT * FROM test;',
    results: {
      columns: [],
      rows: [],
      errorMessage: null,
      isLoading: false,
    },
    isLoading: false,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders correctly with default props', () => {
    const wrapper = shallowMount(QueryView, { props: defaultProps });
    expect(wrapper.exists()).toBe(true);
    expect(wrapper.find('textarea').element.value).toBe(defaultProps.query);
    expect(wrapper.find('button[type="submit"]').text()).toBe('Execute');
    expect(wrapper.find('.results-panel').exists()).toBe(false);
    expect(wrapper.find('.error-panel').exists()).toBe(false);
  });

  it('renders "Executing..." when isLoading is true', () => {
    const wrapper = shallowMount(QueryView, {
      props: { ...defaultProps, isLoading: true },
    });
    expect(wrapper.find('button[type="submit"]').text()).toBe('Executing...');
    expect(wrapper.find('button[type="submit"]').attributes('disabled')).toBeDefined();
  });

  it('emits update:query when textarea content changes', async () => {
    const wrapper = shallowMount(QueryView, { props: defaultProps });
    const textarea = wrapper.find('textarea');
    await textarea.setValue('SELECT 1;');
    expect(wrapper.emitted('update:query')).toBeTruthy();
    expect(wrapper.emitted('update:query')![0]).toEqual(['SELECT 1;']);
  });

  it('emits execute-query when execute button is clicked', async () => {
    const wrapper = shallowMount(QueryView, { props: defaultProps });
    await wrapper.find('form').trigger('submit');
    expect(wrapper.emitted('execute-query')).toBeTruthy();
  });

  it('displays results table when rows are present', () => {
    const mockResults = {
      columns: ['id', 'name'],
      rows: [{ id: 1, name: 'Test1' }, { id: 2, name: 'Test2' }],
      errorMessage: null,
      isLoading: false,
    };
    const wrapper = shallowMount(QueryView, {
      props: { ...defaultProps, results: mockResults },
    });
    expect(wrapper.find('.results-panel').exists()).toBe(true);
    expect(wrapper.findAll('th').map(th => th.text())).toEqual(['id', 'name']);
    expect(wrapper.findAll('tr').length).toBe(3); // 1 header + 2 data rows
  });

  it('displays error panel when errorMessage is present', () => {
    const mockResults = {
      columns: [],
      rows: [],
      errorMessage: 'Syntax error!',
      isLoading: false,
    };
    const wrapper = shallowMount(QueryView, {
      props: { ...defaultProps, results: mockResults },
    });
    expect(wrapper.find('.error-panel').exists()).toBe(true);
    expect(wrapper.find('.error-panel pre').text()).toBe('Syntax error!');
  });

  it('displays "No rows returned" when no rows and no error', () => {
    const mockResults = {
      columns: [],
      rows: [],
      errorMessage: null,
      isLoading: false,
    };
    const wrapper = shallowMount(QueryView, {
      props: { ...defaultProps, results: mockResults, isLoading: false },
    });
    expect(wrapper.text()).toContain('Query executed successfully. No rows returned.');
  });
});
