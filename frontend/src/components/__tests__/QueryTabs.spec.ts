import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { mount } from '@vue/test-utils';
import QueryTabs from '../QueryTabs.vue';
import { useTabsStore } from '../../stores/tabs';
import { useConnectionStore } from '../../stores/connection';
import { useQueryStore } from '../../stores/query';
import { createPinia, setActivePinia } from 'pinia';
import { type QueryResponse } from '@/types/query'; // Import QueryResponse

// Mock Pinia stores
vi.mock('../../stores/tabs', () => ({
  useTabsStore: vi.fn(),
}));
vi.mock('../../stores/connection', () => ({
  useConnectionStore: vi.fn(),
}));
vi.mock('../../stores/query', () => ({
  useQueryStore: vi.fn(),
}));

// Mock QueryView
vi.mock('@/views/QueryView.vue', () => ({
  default: {
    name: 'QueryView',
    props: ['query', 'response', 'isLoading', 'errorMessage'], // Updated props
    emits: ['update:query', 'execute-query'],
    template: '<div>Mock Query View</div>',
  },
}));

describe('QueryTabs.vue', () => {
  let tabsStoreMock: ReturnType<typeof useTabsStore>;
  let connectionStoreMock: ReturnType<typeof useConnectionStore>;
  let queryStoreMock: ReturnType<typeof useQueryStore>;

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();

    const tempPinia = createPinia();
    const tempTabsStore = useTabsStore(tempPinia);
    tabsStoreMock = {
      tabs: [],
      activeTabId: null,
      activeTab: null,
      addTab: vi.fn(),
      closeTab: vi.fn(),
      setActiveTab: vi.fn(),
      $state: tempTabsStore.$state,
      $patch: tempTabsStore.$patch,
      $reset: tempTabsStore.$reset,
      $subscribe: tempTabsStore.$subscribe,
      $onAction: tempTabsStore.$onAction,
      $id: tempTabsStore.$id,
      $dispose: tempTabsStore.$dispose,
    } as unknown as ReturnType<typeof useTabsStore>;

    const tempConnectionStore = useConnectionStore(tempPinia);
    connectionStoreMock = {
      disconnect: vi.fn(),
      isConnected: tempConnectionStore.isConnected,
      isConnecting: tempConnectionStore.isConnecting,
      errorMessage: tempConnectionStore.errorMessage,
      connect: tempConnectionStore.connect,
      $state: tempConnectionStore.$state,
      $patch: tempConnectionStore.$patch,
      $reset: tempConnectionStore.$reset,
      $subscribe: tempConnectionStore.$subscribe,
      $onAction: tempConnectionStore.$onAction,
      $id: tempConnectionStore.$id,
      $dispose: tempConnectionStore.$dispose,
    } as unknown as ReturnType<typeof useConnectionStore>;

    const tempQueryStore = useQueryStore(tempPinia);
    queryStoreMock = {
      executeQuery: vi.fn(),
      setQueryState: vi.fn(),
      response: null, // Default for new structure
      errorMessage: '',
      isLoading: false,
      resultRows: tempQueryStore.resultRows,
      resultColumns: tempQueryStore.resultColumns,
      messages: tempQueryStore.messages,
      $state: tempQueryStore.$state,
      $patch: tempQueryStore.$patch,
      $reset: tempQueryStore.$reset,
      $subscribe: tempQueryStore.$subscribe,
      $onAction: tempQueryStore.$onAction,
      $id: tempQueryStore.$id,
      $dispose: tempQueryStore.$dispose,
    } as unknown as ReturnType<typeof useQueryStore>;

    (useTabsStore as Mock).mockReturnValue(tabsStoreMock);
    (useConnectionStore as Mock).mockReturnValue(connectionStoreMock);
    (useQueryStore as Mock).mockReturnValue(queryStoreMock);
  });

  it('renders correctly with no tabs', () => {
    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    expect(wrapper.exists()).toBe(true);
    expect(wrapper.find('.no-tab-selected').exists()).toBe(true);
  });

  it('adds a default tab on mounted if no tabs exist', () => {
    mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    expect(tabsStoreMock.addTab).toHaveBeenCalledTimes(1);
  });

  it('renders tabs and highlights the active one', async () => {
    tabsStoreMock.tabs = [
      { id: '1', title: 'Query 1', query: '', response: null, isLoading: false, errorMessage: null, isSaved: false },
      { id: '2', title: 'Query 2', query: '', response: null, isLoading: false, errorMessage: null, isSaved: false },
    ];
    tabsStoreMock.activeTabId = '2';
    tabsStoreMock.activeTab = tabsStoreMock.tabs[1];

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick(); // Wait for reactivity

    const tabItems = wrapper.findAll('.tab-item');
    expect(tabItems.length).toBe(2);
    expect(tabItems[0].text()).toContain('Query 1');
    expect(tabItems[0].classes()).not.toContain('active');
    expect(tabItems[1].text()).toContain('Query 2');
    expect(tabItems[1].classes()).toContain('active');
  });

  it('calls setActiveTab when a tab is clicked', async () => {
    tabsStoreMock.tabs = [
      { id: '1', title: 'Query 1', query: '', response: null, isLoading: false, errorMessage: null, isSaved: false },
      { id: '2', title: 'Query 2', query: '', response: null, isLoading: false, errorMessage: null, isSaved: false },
    ];
    tabsStoreMock.activeTabId = '1';
    tabsStoreMock.activeTab = tabsStoreMock.tabs[0];

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    const tabItems = wrapper.findAll('.tab-item');
    await tabItems[1].trigger('click'); // Click Query 2

    expect(tabsStoreMock.setActiveTab).toHaveBeenCalledWith('2');
  });

  it('calls closeTab when close button is clicked', async () => {
    tabsStoreMock.tabs = [
      { id: '1', title: 'Query 1', query: '', response: null, isLoading: false, errorMessage: null, isSaved: false },
    ];
    tabsStoreMock.activeTabId = '1';
    tabsStoreMock.activeTab = tabsStoreMock.tabs[0];

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    await wrapper.find('.close-tab').trigger('click');
    expect(tabsStoreMock.closeTab).toHaveBeenCalledWith('1');
  });

  it('calls addTab when add button is clicked', async () => {
    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.find('.add-tab').trigger('click');
    expect(tabsStoreMock.addTab).toHaveBeenCalledTimes(2); // 1 on mounted, 1 on click
  });

  it('renders QueryView for active tab and passes correct props', async () => {
    const mockResponse: QueryResponse = {
        results: [{ columns: ['id'], rows: [{ id: 1 }] }],
        messages: [],
    };
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT current_time;',
      response: mockResponse,
      isLoading: false,
      errorMessage: null,
      isSaved: false,
    };
    tabsStoreMock.tabs = [mockActiveTab];
    tabsStoreMock.activeTabId = mockActiveTab.id;
    tabsStoreMock.activeTab = mockActiveTab;

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    const queryView = wrapper.findComponent({ name: 'QueryView' }); // Find by component name
    expect(queryView.exists()).toBe(true);
    expect(queryView.props('query')).toBe(mockActiveTab.query);
    expect(queryView.props('response')).toEqual(mockActiveTab.response); // Changed from results
    expect(queryView.props('isLoading')).toBe(mockActiveTab.isLoading); // Changed from mockActiveTab.results.isLoading
    expect(queryView.props('errorMessage')).toBe(mockActiveTab.errorMessage); // New assertion
  });

  it('updates active tab query when QueryView emits update:query', async () => {
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'OLD QUERY;',
      response: null,
      isLoading: false,
      errorMessage: null,
      isSaved: false,
    };
    tabsStoreMock.tabs = [mockActiveTab];
    tabsStoreMock.activeTabId = mockActiveTab.id;
    tabsStoreMock.activeTab = mockActiveTab;

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    const queryView = wrapper.findComponent({ name: 'QueryView' });
    await queryView.vm.$emit('update:query', 'NEW QUERY;');
    expect(mockActiveTab.query).toBe('NEW QUERY;');
  });

  it('executes query and updates active tab results when QueryView emits execute-query', async () => {
    const mockResponse: QueryResponse = {
        results: [{ columns: ['col1'], rows: [{ col1: 'val1' }] }],
        messages: [],
    };
    queryStoreMock.response = mockResponse;

    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT 1;',
      response: null,
      isLoading: false,
      errorMessage: null,
      isSaved: false,
    };
    tabsStoreMock.tabs = [mockActiveTab];
    tabsStoreMock.activeTabId = mockActiveTab.id;
    tabsStoreMock.activeTab = mockActiveTab;

    queryStoreMock.executeQuery.mockResolvedValue(true);

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    const queryView = wrapper.findComponent({ name: 'QueryView' });
    await queryView.vm.$emit('execute-query');

    expect(mockActiveTab.isLoading).toBe(false);
    expect(mockActiveTab.response).toEqual(mockResponse);
    expect(mockActiveTab.errorMessage).toBeNull();
  });

  it('handles query execution error and updates active tab results', async () => {
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT bad_syntax;',
      response: null,
      isLoading: false,
      errorMessage: null,
      isSaved: false,
    };
    tabsStoreMock.tabs = [mockActiveTab];
    tabsStoreMock.activeTabId = mockActiveTab.id;
    tabsStoreMock.activeTab = mockActiveTab;

    queryStoreMock.executeQuery.mockResolvedValue(false);
    queryStoreMock.errorMessage = 'Syntax Error';

    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.vm.$nextTick();

    const queryView = wrapper.findComponent({ name: 'QueryView' });
    await queryView.vm.$emit('execute-query');

    expect(mockActiveTab.isLoading).toBe(false);
    expect(mockActiveTab.response).toBeNull();
    expect(mockActiveTab.errorMessage).toBe('Syntax Error');
  });

  it('calls connectionStore.disconnect when disconnect button is clicked', async () => {
    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.find('header > button').trigger('click');
    expect(connectionStoreMock.disconnect).toHaveBeenCalledTimes(1);
  });
});