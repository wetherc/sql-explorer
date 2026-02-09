import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest'
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
    const tempPinia = createPinia();
    const tempStore = useConnectionStore(tempPinia);

    connectionStoreMock = {
      isConnected: tempStore.isConnected,
      isConnecting: tempStore.isConnecting,
      errorMessage: tempStore.errorMessage,

      connect: vi.fn(),
      disconnect: vi.fn(),

      $state: tempStore.$state,
      $patch: tempStore.$patch,
      $reset: tempStore.$reset,
      $subscribe: tempStore.$subscribe,
      $onAction: tempStore.$onAction,
      $id: tempStore.$id,
      $dispose: tempStore.$dispose,
      // _p: tempStore._p, // Private property, may not be directly accessible or needed
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
