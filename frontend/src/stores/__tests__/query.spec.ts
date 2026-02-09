import { setActivePinia, createPinia } from 'pinia'
import { useQueryStore } from '../query'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/tauri'
import { type QueryResponse } from '@/types/query'

// Mock the tauri invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn()
}))

describe('Query Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // Reset mocks before each test
    vi.mocked(invoke).mockReset()
  })

  it('initializes with correct default state', () => {
    const store = useQueryStore()
    expect(store.response).toBeNull()
    expect(store.isLoading).toBe(false)
    expect(store.errorMessage).toBe('')
    expect(store.resultRows).toEqual([])
    expect(store.resultColumns).toEqual([])
    expect(store.messages).toEqual([])
  })

  it('executes a query and updates state with single result set', async () => {
    const store = useQueryStore()
    const mockResponse: QueryResponse = {
      results: [
        {
          columns: ['id', 'name'],
          rows: [{ id: 1, name: 'Test1' }, { id: 2, name: 'Test2' }]
        }
      ],
      messages: ['Query executed successfully.']
    }
    vi.mocked(invoke).mockResolvedValue(mockResponse)

    const success = await store.executeQuery('SELECT * FROM test')

    expect(success).toBe(true)
    expect(store.isLoading).toBe(false)
    expect(store.errorMessage).toBe('Query executed successfully.')
    expect(store.response).toEqual(mockResponse)
    expect(store.resultRows).toEqual(mockResponse.results[0].rows)
    expect(store.resultColumns).toEqual(mockResponse.results[0].columns)
    expect(store.messages).toEqual(mockResponse.messages)
    expect(invoke).toHaveBeenCalledWith('execute_query', { query: 'SELECT * FROM test' })
  })

  it('executes a query and updates state with multiple result sets', async () => {
    const store = useQueryStore()
    const mockResponse: QueryResponse = {
      results: [
        { columns: ['col1'], rows: [{ col1: 'val1' }] },
        { columns: ['colA', 'colB'], rows: [{ colA: 1, colB: 'a' }] }
      ],
      messages: []
    }
    vi.mocked(invoke).mockResolvedValue(mockResponse)

    const success = await store.executeQuery('SELECT 1; SELECT 2;')

    expect(success).toBe(true)
    expect(store.isLoading).toBe(false)
    expect(store.response).toEqual(mockResponse)
    // Should still only expose the first result set via computed properties
    expect(store.resultRows).toEqual(mockResponse.results[0].rows)
    expect(store.resultColumns).toEqual(mockResponse.results[0].columns)
    expect(store.messages).toEqual([])
  })

  it('executes a query and updates state with messages only', async () => {
    const store = useQueryStore()
    const mockResponse: QueryResponse = {
      results: [],
      messages: ['(0 rows affected)', 'Command(s) completed successfully.']
    }
    vi.mocked(invoke).mockResolvedValue(mockResponse)

    const success = await store.executeQuery('DELETE FROM table;')

    expect(success).toBe(true)
    expect(store.isLoading).toBe(false)
    expect(store.errorMessage).toBe(`(0 rows affected)
Command(s) completed successfully.`)
    expect(store.response).toEqual(mockResponse)
    expect(store.resultRows).toEqual([])
    expect(store.resultColumns).toEqual([])
    expect(store.messages).toEqual(mockResponse.messages)
  })

  it('handles query error from backend', async () => {
    const store = useQueryStore()
    const errorMessage = 'Syntax error near SELECT'
    vi.mocked(invoke).mockRejectedValue(errorMessage)

    const success = await store.executeQuery('SELECT FROM bad_syntax')

    expect(success).toBe(false)
    expect(store.isLoading).toBe(false)
    expect(store.errorMessage).toBe(errorMessage)
    expect(store.response).toBeNull()
    expect(store.resultRows).toEqual([])
    expect(store.resultColumns).toEqual([])
    expect(store.messages).toEqual([])
  })

  it('handles backend response with error messages', async () => {
    const store = useQueryStore();
    const mockErrorResponse: QueryResponse = {
      results: [],
      messages: ["Msg 102, Level 15, State 1, Line 1", "Incorrect syntax near 'BAD'."]
    };
    vi.mocked(invoke).mockResolvedValue(mockErrorResponse);

    const success = await store.executeQuery("SELECT BAD;");

    expect(success).toBe(true); // Should be true if message check is only for explicit 'error' string
    expect(store.isLoading).toBe(false);
    expect(store.errorMessage).toBe(`Msg 102, Level 15, State 1, Line 1
Incorrect syntax near 'BAD'.`);
    expect(store.response).toEqual(mockErrorResponse);
    expect(store.resultRows).toEqual([]);
    expect(store.resultColumns).toEqual([]);
    expect(store.messages).toEqual(mockErrorResponse.messages);
  });

  it('setQueryState correctly updates state', () => {
    const store = useQueryStore()
    const newResponse: QueryResponse = {
      results: [
        { columns: ['a'], rows: [{ a: 1 }] }
      ],
      messages: ['State set.']
    }
    const newErrorMessage = 'Custom error'
    const newIsLoading = true

    store.setQueryState('some query', newResponse, newErrorMessage, newIsLoading)

    expect(store.response).toEqual(newResponse)
    expect(store.errorMessage).toBe(newErrorMessage)
    expect(store.isLoading).toBe(newIsLoading)
  })
})