import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { baseName, findNode, isStatementFile, nodeOfEntry, visibleRows, useFilesStore } =
  await import('@/stores/files')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')

/** One entry as the backend sends it. */
function entry(name: string, kind: 'folder' | 'file' = 'file', root = '/data') {
  return { name, path: `${root}/${name}`, kind }
}

describe('the helpers of the files panel', () => {
  it('keeps the files that hold a statement', () => {
    expect(isStatementFile('report.sql')).toBe(true)
    expect(isStatementFile('REPORT.SQL')).toBe(true)
    expect(isStatementFile('notes.txt')).toBe(true)
    expect(isStatementFile('image.png')).toBe(false)
    expect(isStatementFile('sql')).toBe(false)
  })

  it('names a file without the folders in front of it', () => {
    expect(baseName('/data/reports/a.sql')).toBe('a.sql')
    expect(baseName('C:\\data\\a.sql')).toBe('a.sql')
    expect(baseName('a.sql')).toBe('a.sql')
    expect(baseName('/')).toBe('/')
  })

  it('builds a node from one entry', () => {
    const folder = nodeOfEntry(entry('reports', 'folder'), 0)
    expect(folder).toMatchObject({ name: 'reports', kind: 'folder', depth: 0, loaded: false })
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
    const tabs = useTabsStore()

    await files.openFolder()

    expect(files.hasRoots).toBe(true)
    // The folder stands open, so its entries are rows of the panel.
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports', 'a.sql'])
    // The workspace holds the folder, so the next start reaches it again.
    expect(tabs.fileRoots).toEqual(['/data'])
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
    apiStub.pickFolder.mockRejectedValue({ kind: 'io', message: 'refused', detail: null })
    const files = useFilesStore()

    await files.openFolder()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
    expect(files.loading).toBe(false)
  })

  it('adds one folder once and takes it away again', async () => {
    apiStub.pickFolder.mockResolvedValue('/data')
    const files = useFilesStore()
    const tabs = useTabsStore()

    await files.openFolder()
    await files.openFolder()
    expect(files.roots).toHaveLength(1)

    files.closeRoot('/data')
    expect(files.hasRoots).toBe(false)
    expect(tabs.fileRoots).toEqual([])
  })

  it('opens and closes a folder inside a root', async () => {
    const files = useFilesStore()
    files.restoreRoots(['/data'])
    apiStub.listFolder.mockResolvedValue([entry('reports', 'folder')])
    await files.expand('/data')
    apiStub.listFolder.mockResolvedValue([entry('a.sql', 'file', '/data/reports')])

    await files.expand('/data/reports')
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports', 'a.sql'])

    files.collapse('/data/reports')
    expect(files.rows.map((row) => row.name)).toEqual(['data', 'reports'])
  })

  it('reads the entries of a folder once', async () => {
    const files = useFilesStore()
    files.restoreRoots(['/data'])

    await files.expand('/data')
    files.collapse('/data')
    await files.expand('/data')
    expect(apiStub.listFolder).toHaveBeenCalledTimes(1)

    // A refresh reads them again.
    await files.refresh('/data')
    expect(apiStub.listFolder).toHaveBeenCalledTimes(2)
  })

  it('opens no folder that the panel does not hold', async () => {
    const files = useFilesStore()
    files.restoreRoots(['/data'])
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
    files.restoreRoots(['/data'])
    apiStub.listFolder.mockRejectedValue({ kind: 'configuration', message: 'no', detail: null })

    await files.expand('/data')

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
    expect(files.rows).toHaveLength(1)
  })

  it('opens a file in a tab and brings that tab forward a second time', async () => {
    apiStub.readTextFile.mockResolvedValue('SELECT 1')
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

  it('reports a file that cannot be read', async () => {
    apiStub.readTextFile.mockRejectedValue({ kind: 'io', message: 'gone', detail: null })
    const files = useFilesStore()
    const tabs = useTabsStore()

    await files.openFile('/data/a.sql')

    expect(tabs.tabs).toHaveLength(0)
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('puts the folders of the workspace back into the panel', () => {
    const files = useFilesStore()
    files.restoreRoots(['/data', '/other'])
    expect(files.roots.map((root) => root.name)).toEqual(['data', 'other'])

    // A second restore takes the place of the first.
    files.restoreRoots(['/only'])
    expect(files.roots.map((root) => root.name)).toEqual(['only'])
  })
})
