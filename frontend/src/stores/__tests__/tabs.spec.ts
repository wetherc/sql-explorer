import { setActivePinia, createPinia } from 'pinia';
import { describe, it, expect, beforeEach, vi, type Mock } from 'vitest';
import { useTabsStore } from '../tabs';

// Mock uuidv4
vi.mock('uuid', () => ({
  v4: vi.fn(),
}));

// Import the mocked v4
import { v4 as uuidv4 } from 'uuid';

describe('tabs store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    (uuidv4 as Mock).mockImplementation(() => Math.random().toString()); // Simple mock for unique IDs
  });

  it('should have correct initial state', () => {
    const store = useTabsStore();
    expect(store.tabs).toEqual([]);
    expect(store.activeTabId).toBeNull();
    expect(store.activeTab).toBeNull();
  });

  it('addTab should add a new tab and set it as active', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-1');

    store.addTab();

    expect(store.tabs.length).toBe(1);
    expect(store.tabs[0]!.id).toBe('tab-1');
    expect(store.tabs[0]!.title).toBe('New Query 1');
    expect(store.tabs[0]!.query).toBe('');
    expect(store.activeTabId).toBe('tab-1');
    expect(store.activeTab).toEqual(store.tabs[0]);
  });

  it('addTab should add a new tab with provided content', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-2');

    store.addTab('SELECT * FROM users;');

    expect(store.tabs.length).toBe(1);
    expect(store.tabs[0]!.id).toBe('tab-2');
    expect(store.tabs[0]!.query).toBe('SELECT * FROM users;');
  });

  it('closeTab should remove the specified tab', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-a');
    (uuidv4 as Mock).mockReturnValueOnce('tab-b');
    store.addTab('query A');
    store.addTab('query B'); // activeTabId is now 'tab-b'

    store.closeTab('tab-a');

    expect(store.tabs.length).toBe(1);
    expect(store.tabs[0]!.id).toBe('tab-b');
    expect(store.activeTabId).toBe('tab-b'); // 'tab-b' should remain active
  });

  it('closeTab should activate a neighboring tab if the active tab is closed', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-1');
    (uuidv4 as Mock).mockReturnValueOnce('tab-2');
    (uuidv4 as Mock).mockReturnValueOnce('tab-3');
    store.addTab(); // tab-1 active
    store.addTab(); // tab-2 active
    store.addTab(); // tab-3 active

    expect(store.activeTabId).toBe('tab-3');

    // Close active tab, should activate previous
    store.closeTab('tab-3');
    expect(store.tabs.length).toBe(2);
    expect(store.activeTabId).toBe('tab-2');

    // Close middle tab, should activate previous
    store.closeTab('tab-2');
    expect(store.tabs.length).toBe(1);
    expect(store.activeTabId).toBe('tab-1');
  });

  it('closeTab should set activeTabId to null if last tab is closed', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-only');
    store.addTab(); // 'tab-only' active

    store.closeTab('tab-only');
    expect(store.tabs.length).toBe(0);
    expect(store.activeTabId).toBeNull();
  });

  it('setActiveTab should change the active tab', () => {
    const store = useTabsStore();
    (uuidv4 as Mock).mockReturnValueOnce('tab-x');
    (uuidv4 as Mock).mockReturnValueOnce('tab-y');
    store.addTab(); // tab-x active
    store.addTab(); // tab-y active

    expect(store.activeTabId).toBe('tab-y');

    store.setActiveTab('tab-x');
    expect(store.activeTabId).toBe('tab-x');
    expect(store.activeTab?.id).toBe('tab-x');
  });
});
