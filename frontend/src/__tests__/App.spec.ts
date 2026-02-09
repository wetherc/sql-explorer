import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import App from '../App.vue'
import { useConnectionStore } from '../stores/connection'

// Mock child components
vi.mock('@/views/ConnectionView.vue', () => ({
  default: {
    name: 'ConnectionView',
    template: '<div>Mock Connection View</div>',
  },
}));

vi.mock('@/components/QueryTabs.vue', () => ({
  default: {
    name: 'QueryTabs',
    template: '<div>Mock Query Tabs</div>',
  },
}));

vi.mock('@/components/DbExplorer.vue', () => ({
  default: {
    name: 'DbExplorer',
    template: '<div>Mock DbExplorer</div>',
  },
}));

// Mock Pinia store
vi.mock('@/stores/connection', () => ({
  useConnectionStore: vi.fn(),
}));

describe('App', () => {
  let connectionStoreMock: ReturnType<typeof useConnectionStore>;

  beforeEach(() => {
    connectionStoreMock = {
      isConnected: false,
      isConnecting: false,
      errorMessage: '',
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    (useConnectionStore as vi.Mock).mockReturnValue(connectionStoreMock);
  });

  it('renders ConnectionView by default if not connected', () => {
    const pinia = createPinia();
    connectionStoreMock.isConnected = false;

    const wrapper = mount(App, {
      global: {
        plugins: [pinia],
        stubs: {
          ConnectionView: true,
          QueryTabs: true,
          DbExplorer: true, // Explicitly stub DbExplorer
        },
      },
    });

    expect(wrapper.findComponent({ name: 'ConnectionView' }).exists()).toBe(true);
    expect(wrapper.findComponent({ name: 'QueryTabs' }).exists()).toBe(false);
  });

  it('renders QueryTabs if connected', async () => {
    const pinia = createPinia();
    connectionStoreMock.isConnected = true; // Simulate connected state

    const wrapper = mount(App, {
      global: {
        plugins: [pinia],
        stubs: {
          ConnectionView: true,
          QueryTabs: true,
          DbExplorer: true, // Explicitly stub DbExplorer
        },
      },
    });
    await wrapper.vm.$nextTick(); // Wait for state change to propagate

    expect(wrapper.findComponent({ name: 'ConnectionView' }).exists()).toBe(false);
    expect(wrapper.findComponent({ name: 'QueryTabs' }).exists()).toBe(true);
  });
});
