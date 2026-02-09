import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount } from '@vue/test-utils';
import QueryTabs from '../QueryTabs.vue';
import { useTabsStore } from '../../stores/tabs';
import { useConnectionStore } from '../../stores/connection';
import { useQueryStore } from '../../stores/query';
import { createPinia, setActivePinia } from 'pinia';

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
    props: ['query', 'results', 'isLoading'],
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

    tabsStoreMock = {
      tabs: [],
      activeTabId: null,
      activeTab: null,
      addTab: vi.fn(),
      closeTab: vi.fn(),
      setActiveTab: vi.fn(),
    };
    connectionStoreMock = {
      disconnect: vi.fn(),
    };
    queryStoreMock = {
      executeQuery: vi.fn(),
      setQueryState: vi.fn(),
      resultColumns: [],
      resultRows: [],
      errorMessage: '',
      isLoading: false,
    };

    (useTabsStore as vi.Mock).mockReturnValue(tabsStoreMock);
    (useConnectionStore as vi.Mock).mockReturnValue(connectionStoreMock);
    (useQueryStore as vi.Mock).mockReturnValue(queryStoreMock);
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
      { id: '1', title: 'Query 1', query: '', results: { columns: [], rows: [], errorMessage: null, isLoading: false }, isSaved: false },
      { id: '2', title: 'Query 2', query: '', results: { columns: [], rows: [], errorMessage: null, isLoading: false }, isSaved: false },
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
      { id: '1', title: 'Query 1', query: '', results: { columns: [], rows: [], errorMessage: null, isLoading: false }, isSaved: false },
      { id: '2', title: 'Query 2', query: '', results: { columns: [], rows: [], errorMessage: null, isLoading: false }, isSaved: false },
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
      { id: '1', title: 'Query 1', query: '', results: { columns: [], rows: [], errorMessage: null, isLoading: false }, isSaved: false },
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
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT current_time;',
      results: { columns: [], rows: [], errorMessage: null, isLoading: false },
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
    expect(queryView.props('results')).toEqual(mockActiveTab.results);
    expect(queryView.props('isLoading')).toBe(mockActiveTab.results.isLoading);
  });

  it('updates active tab query when QueryView emits update:query', async () => {
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'OLD QUERY;',
      results: { columns: [], rows: [], errorMessage: null, isLoading: false },
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
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT 1;',
      results: { columns: [], rows: [], errorMessage: null, isLoading: false },
      isSaved: false,
    };
    tabsStoreMock.tabs = [mockActiveTab];
    tabsStoreMock.activeTabId = mockActiveTab.id;
    tabsStoreMock.activeTab = mockActiveTab;

    queryStoreMock.executeQuery.mockResolvedValue(true);
    queryStoreMock.resultColumns = ['col1'];
    queryStoreMock.resultRows = [{ col1: 'val1' }];

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

    expect(mockActiveTab.results.isLoading).toBe(false);
    expect(mockActiveTab.results.columns).toEqual(['col1']);
    expect(mockActiveTab.results.rows).toEqual([{ col1: 'val1' }]);
    expect(mockActiveTab.results.errorMessage).toBeNull();
  });

  it('handles query execution error and updates active tab results', async () => {
    const mockActiveTab = {
      id: 'active',
      title: 'Active Query',
      query: 'SELECT bad_syntax;',
      results: { columns: [], rows: [], errorMessage: null, isLoading: false },
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

    expect(mockActiveTab.results.isLoading).toBe(false);
    expect(mockActiveTab.results.columns).toEqual([]);
    expect(mockActiveTab.results.rows).toEqual([]);
    expect(mockActiveTab.results.errorMessage).toBe('Syntax Error');
  });

  it('calls connectionStore.disconnect when disconnect button is clicked', async () => {
    const wrapper = mount(QueryTabs, {
      global: {
        stubs: {
          QueryView: true,
        },
      },
    });
    await wrapper.find('header > button').trigger('click'); // The first button in the header should be the disconnect button
    expect(connectionStoreMock.disconnect).toHaveBeenCalledTimes(1);
  });
});
