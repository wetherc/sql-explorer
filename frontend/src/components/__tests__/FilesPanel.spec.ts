import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const FilesPanel = (await import('@/components/FilesPanel.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useFilesStore } = await import('@/stores/files')
const { useTabsStore } = await import('@/stores/tabs')

/** One entry as the backend sends it. */
function entry(name: string, kind: 'folder' | 'file' = 'file', root = '/data') {
  return { name, path: `${root}/${name}`, kind }
}

/** Mounts the panel with one folder already open. */
async function mountWithRoot() {
  const wrapper = mountWithPlugins(FilesPanel)
  useFilesStore().restoreRoots(['/data'])
  await settle()
  return wrapper
}

describe('FilesPanel', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.listFolder.mockResolvedValue([])
  })

  it('points at the folder dialog while it holds no folder', async () => {
    apiStub.pickFolder.mockResolvedValue(null)
    const wrapper = mountWithPlugins(FilesPanel)
    await settle()

    expect(wrapper.text()).toContain('No folders yet')
    await wrapper.find('[data-test="files-empty-open"]').trigger('click')
    await settle()
    expect(apiStub.pickFolder).toHaveBeenCalled()
  })

  it('opens a folder from the button of the header', async () => {
    apiStub.pickFolder.mockResolvedValue('/data')
    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    const wrapper = mountWithPlugins(FilesPanel)
    await settle()

    await wrapper.find('[data-test="files-open-folder"]').trigger('click')
    await settle()

    const rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows.map((row) => row.text())).toEqual(['data', 'a.sql'])
  })

  it('opens and closes a folder when its row is clicked', async () => {
    apiStub.listFolder.mockResolvedValue([entry('reports', 'folder')])
    const wrapper = await mountWithRoot()

    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(2)

    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(1)
  })

  it('opens a file in a tab, from a click and from the Enter key', async () => {
    apiStub.listFolder.mockResolvedValue([entry('a.sql'), entry('b.sql')])
    apiStub.readTextFile.mockResolvedValue('SELECT 1')
    const wrapper = await mountWithRoot()
    const tabs = useTabsStore()

    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()

    const rows = wrapper.findAll('[data-test="file-row"]')
    await rows[1]!.trigger('click')
    await settle()
    expect(tabs.tabs).toHaveLength(1)

    await rows[2]!.trigger('keydown.enter')
    await settle()
    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.tabs[1]?.filePath).toBe('/data/b.sql')
  })

  it('takes a folder out of the panel from the mark of its row', async () => {
    const wrapper = await mountWithRoot()

    await wrapper.find('[data-test="close-root"]').trigger('click')
    await settle()

    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(0)
    expect(wrapper.text()).toContain('No folders yet')
  })

  it('marks a folder that stands open for a reader', async () => {
    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    const wrapper = await mountWithRoot()
    const root = wrapper.find('[data-test="file-row"]')
    expect(root.attributes('aria-expanded')).toBe('false')

    await root.trigger('click')
    await settle()

    expect(wrapper.find('[data-test="file-row"]').attributes('aria-expanded')).toBe('true')
    // A file reports no state of its own, because it never opens.
    const file = wrapper.findAll('[data-test="file-row"]')[1]!
    expect(file.attributes('aria-expanded')).toBeUndefined()
  })
})
