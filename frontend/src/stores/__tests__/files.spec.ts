import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { baseName, findNode, nodeOfEntry, visibleRows, useFilesStore } =
  await import('@/stores/files')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')

/** One entry as the backend sends it. */
function entry(name: string, entryType: 'folder' | 'file' = 'file', root = '/data') {
  return { name, path: `${root}/${name}`, entryType }
}

describe('the helpers of the files panel', () => {
  it('names a file without the folders in front of it', () => {
    expect(baseName('/data/reports/a.sql')).toBe('a.sql')
    expect(baseName('C:\\data\\a.sql')).toBe('a.sql')
    expect(baseName('a.sql')).toBe('a.sql')
    expect(baseName('/')).toBe('/')
  })

  it('builds a node from one entry', () => {
    const folder = nodeOfEntry(entry('reports', 'folder'), 0)
    expect(folder).toMatchObject({ name: 'reports', entryType: 'folder', depth: 0, loaded: false })
    expect(folder.children).toEqual([])

    // A file holds no children at all.
    expect(nodeOfEntry(entry('a.sql'), 1).children).toBeUndefined()
  })

  it('finds a node wherever it stands in the tree', () => {
    const tree = [
      {
        ...nodeOfEntry(entry('reports', 'folder'), 0),
        children: [nodeOfEntry(entry('a.sql', 'file', '/data/reports'), 1)],
      },
    ]

    expect(findNode(tree, '/data/reports/a.sql')?.name).toBe('a.sql')
    expect(findNode(tree, '/data/reports')?.name).toBe('reports')
    expect(findNode(tree, '/nowhere')).toBeUndefined()
  })

  it('draws the children of a folder that stands open alone', () => {
    const child = nodeOfEntry(entry('a.sql', 'file', '/data/reports'), 1)
    const tree = [{ ...nodeOfEntry(entry('reports', 'folder'), 0), children: [child] }]

    expect(visibleRows(tree, new Set())).toHaveLength(1)
    expect(visibleRows(tree, new Set(['/data/reports']))).toHaveLength(2)
  })
})

describe('files store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.listFolder.mockResolvedValue([])
  })

  it('opens a folder that the user chose and reads its entries', async () => {
    apiStub.pickFolder.mockResolvedValue('/data')
    apiStub.listFolder.mockResolvedValue([
      entry('reports', 'folder'),
      entry('a.sql'),
      entry('image.png'),
    ])
    const files = useFilesStore()

    await files.openFolder()

    expect(files.hasRoots).toBe(true)
    // The folder stands open, so every entry of it is a row of the panel.
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports', 'a.sql', 'image.png'])
    expect(files.loading).toBe(false)
  })

  it('holds the panel as it is when the user closed the dialog', async () => {
    apiStub.pickFolder.mockResolvedValue(null)
    const files = useFilesStore()

    await files.openFolder()

    expect(files.hasRoots).toBe(false)
    expect(apiStub.listFolder).not.toHaveBeenCalled()
  })

  it('reports a folder that cannot be opened', async () => {
    apiStub.pickFolder.mockRejectedValue({ category: 'io', message: 'refused', detail: null })
    const files = useFilesStore()

    await files.openFolder()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
    expect(files.loading).toBe(false)
  })

  it('adds one folder once and takes it away again', async () => {
    apiStub.pickFolder.mockResolvedValue('/data')
    apiStub.closeFolder.mockResolvedValue(undefined)
    const files = useFilesStore()

    await files.openFolder()
    await files.openFolder()
    expect(files.roots).toHaveLength(1)

    await files.closeRoot('/data')
    expect(files.hasRoots).toBe(false)
    // The backend drops the folder as well, so no path under it is
    // reachable any more.
    expect(apiStub.closeFolder).toHaveBeenCalledWith('/data')
  })

  it('opens and closes a folder inside a root', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockResolvedValue([entry('reports', 'folder')])
    await files.expand('/data')
    apiStub.listFolder.mockResolvedValue([entry('a.sql', 'file', '/data/reports')])

    await files.expand('/data/reports')
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports', 'a.sql'])

    files.collapse('/data/reports')
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports'])
  })

  it('reads the entries of a folder again each time it opens', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()

    await files.expand('/data')
    files.collapse('/data')
    await files.expand('/data')
    expect(apiStub.listFolder).toHaveBeenCalledTimes(2)

    // A refresh reads them again.
    await files.refresh('/data')
    expect(apiStub.listFolder).toHaveBeenCalledTimes(3)

    // An open while a read runs starts no second read.
    let answer: (value: unknown) => void = () => {}
    apiStub.listFolder.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const reading = files.refresh('/data')
    await files.expand('/data')
    answer([])
    await reading
    expect(apiStub.listFolder).toHaveBeenCalledTimes(4)
  })

  it('keeps the failure of a folder read and clears it on the next read', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockRejectedValueOnce({ category: 'io', message: 'denied', detail: null })
    await files.expand('/data')
    const root = files.roots[0]!
    expect(root.error).toBe('denied')
    expect(root.loaded).toBe(false)
    expect(useUiStore().notices[0]?.level).toBe('error')

    apiStub.listFolder.mockResolvedValue([])
    await files.refresh('/data')
    expect(files.roots[0]!.error).toBeNull()
    expect(files.roots[0]!.loaded).toBe(true)
  })

  it('reads the open folders below a refreshed folder again', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockResolvedValue([entry('reports', 'folder'), entry('shut', 'folder')])
    await files.expand('/data')
    apiStub.listFolder.mockResolvedValue([entry('a.sql', 'file', '/data/reports')])
    await files.expand('/data/reports')

    apiStub.listFolder.mockImplementation(async (path: string) =>
      path === '/data'
        ? [entry('reports', 'folder'), entry('shut', 'folder')]
        : [entry('a.sql', 'file', '/data/reports'), entry('b.sql', 'file', '/data/reports')],
    )
    await files.refresh('/data')
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports', 'a.sql', 'b.sql', 'shut'])
    // A closed folder waits for its own expand.
    expect(apiStub.listFolder).not.toHaveBeenCalledWith('/data/shut')
  })

  it('reads the open folders below a refreshed folder side by side', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockResolvedValue([entry('slow', 'folder'), entry('quick', 'folder')])
    await files.expand('/data')
    apiStub.listFolder.mockResolvedValue([])
    await files.expand('/data/slow')
    await files.expand('/data/quick')

    let releaseQuick: (value: ReturnType<typeof entry>[]) => void = () => {}
    apiStub.listFolder.mockImplementation((path: string) => {
      if (path === '/data') {
        return Promise.resolve([entry('slow', 'folder'), entry('quick', 'folder')])
      }
      // The slow folder never answers.
      return path === '/data/slow'
        ? new Promise(() => {})
        : new Promise((resolve) => {
            releaseQuick = resolve
          })
    })
    void files.refresh('/data')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'slow', 'quick'])

    releaseQuick([entry('a.sql', 'file', '/data/quick')])
    await new Promise((resolve) => setTimeout(resolve, 0))
    // The panel sees the write into the child, which is the reactive form.
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'slow', 'quick', 'a.sql'])
    expect(files.rows[2]?.loading).toBe(false)
  })

  it('reads a folder again while an older read of it runs', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    let releaseFirst: (value: ReturnType<typeof entry>[]) => void = () => {}
    apiStub.listFolder.mockReturnValueOnce(
      new Promise((resolve) => {
        releaseFirst = resolve
      }),
    )
    const first = files.expand('/data')

    apiStub.listFolder.mockResolvedValue([entry('new.sql')])
    await files.refresh('/data')
    releaseFirst([entry('old.sql')])
    await first
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'new.sql'])
    expect(files.roots[0]?.loading).toBe(false)
  })

  it('says nothing about a failure of a read that a refresh passed', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    let refuseFirst: (error: unknown) => void = () => {}
    apiStub.listFolder.mockReturnValueOnce(
      new Promise((_resolve, reject) => {
        refuseFirst = reject
      }),
    )
    const first = files.expand('/data')

    apiStub.listFolder.mockResolvedValue([entry('new.sql')])
    await files.refresh('/data')
    refuseFirst({ category: 'io', message: 'gone', detail: null })
    await first
    expect(useUiStore().notices).toEqual([])
  })

  it('opens no folder that the panel does not hold', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    await files.expand('/data')

    // A file is not a folder, and a path outside the panel is neither.
    await files.expand('/data/a.sql')
    await files.expand('/nowhere')
    await files.refresh('/data/a.sql')
    await files.refresh('/nowhere')

    expect(apiStub.listFolder).toHaveBeenCalledTimes(1)
  })

  it('reports a folder whose entries cannot be read', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await files.restoreRoots()
    apiStub.listFolder.mockRejectedValue({ category: 'configuration', message: 'no', detail: null })

    await files.expand('/data')

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
    expect(files.rows).toHaveLength(1)
  })

  it('opens a file in a tab and brings that tab forward a second time', async () => {
    apiStub.readTextFile.mockResolvedValue({ contents: 'SELECT 1', encoding: 'utf8' })
    const files = useFilesStore()
    const tabs = useTabsStore()
    const other = tabs.add()

    await files.openFile('/data/a.sql')

    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.activeTab).toMatchObject({
      title: 'a.sql',
      query: 'SELECT 1',
      filePath: '/data/a.sql',
    })

    tabs.activate(other.id)
    await files.openFile('/data/a.sql')
    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.activeTab?.filePath).toBe('/data/a.sql')
    expect(apiStub.readTextFile).toHaveBeenCalledTimes(1)
  })

  it('opens one tab for two quick opens of one file, in its encoding', async () => {
    let answer: (value: unknown) => void = () => {}
    apiStub.readTextFile.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const files = useFilesStore()
    const tabs = useTabsStore()
    const first = files.openFile('/data/a.sql')
    const second = files.openFile('/data/a.sql')
    answer({ contents: 'SELECT 1', encoding: 'utf16le' })
    await Promise.all([first, second])
    expect(tabs.tabs).toHaveLength(1)
    expect(tabs.tabs[0]?.encoding).toBe('utf16le')
    expect(apiStub.readTextFile).toHaveBeenCalledTimes(1)
  })

  it('brings forward a tab that opened the file during the read', async () => {
    let answer: (value: unknown) => void = () => {}
    apiStub.readTextFile.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const files = useFilesStore()
    const tabs = useTabsStore()
    const reading = files.openFile('/data/a.sql')
    const held = tabs.add({ filePath: '/data/a.sql' })
    tabs.add()
    answer({ contents: 'SELECT 1', encoding: 'utf8' })
    await reading
    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.activeTabId).toBe(held.id)
  })

  it('reports a file that cannot be read', async () => {
    apiStub.readTextFile.mockRejectedValue({ category: 'io', message: 'gone', detail: null })
    const files = useFilesStore()
    const tabs = useTabsStore()

    await files.openFile('/data/a.sql')

    expect(tabs.tabs).toHaveLength(0)
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('opens a file that the user chose and leaves its folder out of the panel', async () => {
    apiStub.openStatementFile.mockResolvedValue({
      path: '/data/reports/daily.sql',
      contents: 'SELECT 1',
    })
    const files = useFilesStore()
    const tabs = useTabsStore()

    await files.openFileFromDialog()

    expect(tabs.activeTab).toMatchObject({
      title: 'daily.sql',
      query: 'SELECT 1',
      filePath: '/data/reports/daily.sql',
    })
    // The user accepted one file, so the backend admits that file alone and
    // the panel lists no folder.
    expect(files.roots).toEqual([])
    expect(apiStub.listFolder).not.toHaveBeenCalled()
    expect(files.loading).toBe(false)
  })

  it('brings a file that a tab already holds forward', async () => {
    apiStub.openStatementFile.mockResolvedValue({ path: '/data/a.sql', contents: 'SELECT 1' })
    const files = useFilesStore()
    const tabs = useTabsStore()
    await files.openFileFromDialog()
    const other = tabs.add()

    await files.openFileFromDialog()

    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.activeTab?.id).not.toBe(other.id)
    expect(tabs.activeTab?.filePath).toBe('/data/a.sql')
  })

  it('holds the panel as it is when the user closed the file dialog', async () => {
    apiStub.openStatementFile.mockResolvedValue(null)
    const files = useFilesStore()
    const tabs = useTabsStore()

    await files.openFileFromDialog()

    expect(tabs.tabs).toHaveLength(0)
    expect(files.hasRoots).toBe(false)
  })

  it('reports a file of the dialog that cannot be read', async () => {
    apiStub.openStatementFile.mockRejectedValue({ category: 'io', message: 'gone', detail: null })
    const files = useFilesStore()

    await files.openFileFromDialog()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
    expect(files.loading).toBe(false)
  })

  it('puts the folders that the backend records into the panel', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockResolvedValue(['/data', '/other'])
    await files.restoreRoots()
    expect(files.roots.map((root) => root.name)).toEqual(['data', 'other'])

    // A second restore takes the place of the first.
    apiStub.fileRoots.mockResolvedValue(['/only'])
    await files.restoreRoots()
    expect(files.roots.map((root) => root.name)).toEqual(['only'])
  })

  it('reports a record of folders that cannot be read', async () => {
    const files = useFilesStore()
    apiStub.fileRoots.mockRejectedValue({ category: 'io', message: 'gone', detail: null })

    await files.restoreRoots()

    expect(files.hasRoots).toBe(false)
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('reports a folder that the backend cannot close', async () => {
    apiStub.fileRoots.mockResolvedValue(['/data'])
    apiStub.closeFolder.mockRejectedValue({ category: 'io', message: 'no', detail: null })
    const files = useFilesStore()
    await files.restoreRoots()

    await files.closeRoot('/data')

    expect(files.hasRoots).toBe(false)
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })
})
