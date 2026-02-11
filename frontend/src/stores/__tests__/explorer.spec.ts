import { setActivePinia, createPinia } from 'pinia';
import { describe, it, expect, beforeEach, vi, type Mock } from 'vitest';
import { useExplorerStore } from '../explorer';
import { invoke } from '@tauri-apps/api/tauri';

// Mock the tauri API invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}));

describe('explorer store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    // Reset the mock before each test
    vi.clearAllMocks();
  });

  it('should have correct initial state', () => {
    const store = useExplorerStore();
    expect(store.databases).toEqual([]);
    expect(store.schemas).toEqual([]);
    expect(store.tables).toEqual([]);
    expect(store.columns).toEqual([]);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
  });

  it('fetchDatabases should update state on success', async () => {
    const store = useExplorerStore();
    const mockDatabases = [{ name: 'db1' }, { name: 'db2' }];
    (invoke as any).mockResolvedValue // TODO: Fix TS2339(mockDatabases);

    const promise = store.fetchDatabases();
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_databases');
    expect(store.databases).toEqual(mockDatabases);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
  });

  it('fetchDatabases should set error on failure', async () => {
    const store = useExplorerStore();
    const errorMessage = 'Failed to fetch databases';
    (invoke as any).mockRejectedValue // TODO: Fix TS2339(errorMessage);

    const promise = store.fetchDatabases();
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_databases');
    expect(store.databases).toEqual([]); // Should remain empty or clear existing
    expect(store.loading).toBe(false);
    expect(store.error).toBe(errorMessage);
  });

  it('fetchSchemas should update state on success', async () => {
    const store = useExplorerStore();
    const mockSchemas = [{ name: 'dbo' }, { name: 'guest' }];
    (invoke as any).mockResolvedValue // TODO: Fix TS2339(mockSchemas);

    const promise = store.fetchSchemas();
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_schemas');
    expect(store.schemas).toEqual(mockSchemas);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
  });

  it('fetchSchemas should set error on failure', async () => {
    const store = useExplorerStore();
    const errorMessage = 'Failed to fetch schemas';
    (invoke as any).mockRejectedValue // TODO: Fix TS2339(errorMessage);

    const promise = store.fetchSchemas();
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_schemas');
    expect(store.schemas).toEqual([]);
    expect(store.loading).toBe(false);
    expect(store.error).toBe(errorMessage);
  });

  it('fetchTables should update state on success', async () => {
    const store = useExplorerStore();
    const mockTables = [{ name: 'table1' }, { name: 'table2' }];
    (invoke as any).mockResolvedValue // TODO: Fix TS2339(mockTables);

    const promise = store.fetchTables('dbo');
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_tables', { schemaName: 'dbo' });
    expect(store.tables).toEqual(mockTables);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
  });

  it('fetchTables should set error on failure', async () => {
    const store = useExplorerStore();
    const errorMessage = 'Failed to fetch tables';
    (invoke as any).mockRejectedValue // TODO: Fix TS2339(errorMessage);

    const promise = store.fetchTables('dbo');
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_tables', { schemaName: 'dbo' });
    expect(store.tables).toEqual([]);
    expect(store.loading).toBe(false);
    expect(store.error).toBe(errorMessage);
  });

  it('fetchColumns should update state on success', async () => {
    const store = useExplorerStore();
    const mockColumns = [{ name: 'id', data_type: 'int' }, { name: 'name', data_type: 'varchar' }];
    (invoke as any).mockResolvedValue // TODO: Fix TS2339(mockColumns);

    const promise = store.fetchColumns('dbo', 'mytable');
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_columns', { schemaName: 'dbo', tableName: 'mytable' });
    expect(store.columns).toEqual(mockColumns);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
  });

  it('fetchColumns should set error on failure', async () => {
    const store = useExplorerStore();
    const errorMessage = 'Failed to fetch columns';
    (invoke as any).mockRejectedValue // TODO: Fix TS2339(errorMessage);

    const promise = store.fetchColumns('dbo', 'mytable');
    expect(store.loading).toBe(true);
    await promise;

    expect(invoke).toHaveBeenCalledWith('list_columns', { schemaName: 'dbo', tableName: 'mytable' });
    expect(store.columns).toEqual([]);
    expect(store.loading).toBe(false);
    expect(store.error).toBe(errorMessage);
  });
});



