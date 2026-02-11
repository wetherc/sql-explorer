import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import PrimeVue from 'primevue/config'
import ToastService from 'primevue/toastservice'
import SaveConnectionDialog from '../SaveConnectionDialog.vue'
import { AuthType, DbType } from '@/types/savedConnection'
import { useToast } from 'primevue/usetoast'

// Mock the useToast composable
vi.mock('primevue/usetoast')

describe('SaveConnectionDialog.vue', () => {
  const connectionProps = {
    db_type: DbType.Mssql,
    server: 'test-server',
    database: 'test-db',
    auth_type: AuthType.Sql,
    user: 'test-user',
  }

  const mockToastAdd = vi.fn();

  const mountOptions = (visible = true) => ({
    props: {
      visible: visible,
      connection: connectionProps,
      password: 'test-password',
    },
    global: {
      plugins: [PrimeVue, ToastService],
      stubs: {
        Dialog: {
          template: '<div v-if="visible" class="p-dialog-stub"><slot /><slot name="footer" /></div>',
          props: ['visible'],
        },
      },
    },
  })

  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useToast).mockReturnValue({ add: mockToastAdd } as any);
  })

  it('renders correctly when visible is true', () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    expect(wrapper.find('.p-dialog-stub').exists()).toBe(true)
  })

  it('does not render when visible is false', () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(false))
    expect(wrapper.find('.p-dialog-stub').exists()).toBe(false)
  })

  it('emits update:visible when Cancel button is clicked', async () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    // The cancel button is the first button in the footer
    const cancelButton = wrapper.findAll('button').find(b => b.text() === 'Cancel');
    if (cancelButton) {
      await cancelButton.trigger('click');
    }
    const emitted = wrapper.emitted('update:visible');
    expect(emitted).toBeTruthy(); // Ensure the event was emitted
    expect(emitted![0]).toEqual([false]);
  })

  it('emits a save event with name when save button is clicked', async () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    await wrapper.find('input[type="text"]').setValue('My Test Connection')
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted().save).toBeTruthy()
    expect(wrapper.emitted().save![0]).toEqual(['My Test Connection', undefined])
  })

  it('emits a save event with name and password when checkbox is checked', async () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    await wrapper.find('input[type="text"]').setValue('My Test Connection')
    await wrapper.find('input[type="checkbox"]').setValue(true)
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted().save).toBeTruthy()
    expect(wrapper.emitted().save![0]).toEqual(['My Test Connection', 'test-password'])
  })

  it('shows a toast if connection name is empty', async () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    await wrapper.find('form').trigger('submit.prevent')

    expect(mockToastAdd).toHaveBeenCalledWith(expect.objectContaining({
      severity: 'warn',
      summary: 'Validation Error',
    }))
    expect(wrapper.emitted().save).toBeFalsy()
  })

  it('shows save password checkbox only for SQL auth', async () => {
    const wrapper = mount(SaveConnectionDialog, mountOptions(true))
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(true)

    await wrapper.setProps({
      connection: { ...connectionProps, auth_type: AuthType.Integrated },
    })
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false)
  })
})