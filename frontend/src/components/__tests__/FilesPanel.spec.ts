import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const FilesPanel = (await import('@/components/FilesPanel.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useFilesStore } = await import('@/stores/files')
const { useTabsStore } = await import('@/stores/tabs')

/** One entry as the backend sends it. */
function entry(name: string, entryType: 'folder' | 'file' = 'file', root = '/data') {
  return { name, path: `${root}/${name}`, entryType }
}

/** Mounts the panel with one folder already open. */
async function mountWithRoot() {
  const wrapper = mountWithPlugins(FilesPanel)
  apiStub.fileRoots.mockResolvedValue(['/data'])
  await useFilesStore().restoreRoots()
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
    apiStub.readTextFile.mockResolvedValue({ contents: 'SELECT 1', encoding: 'utf8' })
    const wrapper = await mountWithRoot()
    const tabs = useTabsStore()

    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()

    const rows = wrapper.findAll('[data-test="file-row"]')
    await rows[1]!.trigger('click')
    await settle()
    expect(tabs.tabs).toHaveLength(1)

    await rows[2]!.trigger('keydown', { key: 'Enter' })
    await settle()
    expect(tabs.tabs).toHaveLength(2)
    expect(tabs.tabs[1]?.filePath).toBe('/data/b.sql')
  })

  it('walks the rows with the arrows and holds one tab stop', async () => {
    apiStub.listFolder.mockImplementation((path: string) =>
      Promise.resolve(
        path === '/data'
          ? [entry('reports', 'folder'), entry('a.sql')]
          : [entry('c.sql', 'file', '/data/reports')],
      ),
    )
    const wrapper = await mountWithRoot()

    // The right arrow opens the root, and the tree holds one tab stop.
    await wrapper.find('[data-test="file-row"]').trigger('keydown', { key: 'ArrowRight' })
    await settle()
    let rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows).toHaveLength(3)
    expect(rows.map((row) => row.attributes('tabindex'))).toEqual(['0', '-1', '-1'])

    // The down arrow moves the stop, and the row it moves to takes it.
    await rows[0]!.trigger('keydown', { key: 'ArrowDown' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows.map((row) => row.attributes('tabindex'))).toEqual(['-1', '0', '-1'])

    // The right arrow on a closed folder opens it and the next one steps in.
    await rows[1]!.trigger('keydown', { key: 'ArrowRight' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows).toHaveLength(4)
    await rows[1]!.trigger('keydown', { key: 'ArrowRight' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows[2]!.attributes('tabindex')).toBe('0')

    // The left arrow on a row that holds nothing open steps out to its
    // folder, and the next one closes that folder.
    await rows[2]!.trigger('keydown', { key: 'ArrowLeft' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows[1]!.attributes('tabindex')).toBe('0')
    await rows[1]!.trigger('keydown', { key: 'ArrowLeft' })
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(3)

    // End and Home reach the last row and the first one.
    rows = wrapper.findAll('[data-test="file-row"]')
    await rows[0]!.trigger('keydown', { key: 'End' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows[2]!.attributes('tabindex')).toBe('0')
    await rows[2]!.trigger('keydown', { key: 'Home' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows[0]!.attributes('tabindex')).toBe('0')

    // The up arrow at the first row and a key the tree does not hold change
    // nothing.
    await rows[0]!.trigger('keydown', { key: 'ArrowUp' })
    await rows[0]!.trigger('keydown', { key: 'a' })
    await settle()
    rows = wrapper.findAll('[data-test="file-row"]')
    expect(rows[0]!.attributes('tabindex')).toBe('0')
    // The left arrow at the root closes it, and a second one steps nowhere
    // because a root holds no folder above it.
    await rows[0]!.trigger('keydown', { key: 'ArrowLeft' })
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(1)
    await wrapper.find('[data-test="file-row"]').trigger('keydown', { key: 'ArrowLeft' })
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(1)
  })

  it('opens a folder with the space key', async () => {
    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    const wrapper = await mountWithRoot()

    await wrapper.find('[data-test="file-row"]').trigger('keydown', { key: ' ' })
    await settle()

    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(2)
  })

  it('marks the row the user reached as the tab stop', async () => {
    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    const wrapper = await mountWithRoot()
    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()

    const rows = wrapper.findAll('[data-test="file-row"]')
    await rows[1]!.trigger('focus')
    await settle()
    expect(
      wrapper.findAll('[data-test="file-row"]').map((row) => row.attributes('tabindex')),
    ).toEqual(['-1', '0'])
  })

  it('takes a folder out of the panel with the Delete key', async () => {
    const wrapper = await mountWithRoot()

    await wrapper.find('[data-test="file-row"]').trigger('keydown', { key: 'Delete' })
    await settle()

    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(0)
    expect(wrapper.find('[data-test="file-row"]').exists()).toBe(false)
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

describe('FilesPanel notes and refresh', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.listFolder.mockResolvedValue([])
  })

  it('says when an open folder has no entries', async () => {
    const wrapper = await mountWithRoot()
    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()
    expect(wrapper.find('[data-test="file-empty"]').text()).toBe('Empty folder')
  })

  it('shows a failed read with a Retry button that reads the folder again', async () => {
    apiStub.listFolder.mockRejectedValue({ category: 'io', message: 'Permission denied.' })
    const wrapper = await mountWithRoot()
    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()
    expect(wrapper.find('[data-test="file-error"]').text()).toContain('Permission denied.')

    apiStub.listFolder.mockResolvedValue([entry('a.sql')])
    await wrapper.find('[data-test="file-retry"]').trigger('click')
    await settle()
    expect(wrapper.find('[data-test="file-error"]').exists()).toBe(false)
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(2)
  })

  it('reads every folder again from the Refresh button', async () => {
    const wrapper = await mountWithRoot()
    await wrapper.find('[data-test="file-row"]').trigger('click')
    await settle()
    apiStub.listFolder.mockClear()
    apiStub.listFolder.mockResolvedValue([entry('new.sql')])
    await wrapper.find('[data-test="files-refresh"]').trigger('click')
    await settle()
    expect(apiStub.listFolder).toHaveBeenCalledWith('/data')
    expect(wrapper.text()).toContain('new.sql')
  })

  it('moves the focus to the next folder when Delete takes one away', async () => {
    const wrapper = mountWithPlugins(FilesPanel)
    apiStub.fileRoots.mockResolvedValue(['/one', '/two'])
    await useFilesStore().restoreRoots()
    await settle()
    const rootRow = (name: string) =>
      wrapper.findAll('[data-test="file-row"]').find((row) => row.text() === name)!

    await rootRow('one').trigger('keydown', { key: 'Delete' })
    await settle()
    expect(document.activeElement?.textContent).toContain('two')

    await rootRow('two').trigger('keydown', { key: 'Delete' })
    await settle()
    expect(wrapper.findAll('[data-test="file-row"]')).toHaveLength(0)
  })
})
