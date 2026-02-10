import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import SaveConnectionDialog from '../SaveConnectionDialog.vue'
import { AuthType, DbType } from '@/types/savedConnection'

describe('SaveConnectionDialog.vue', () => {
  const connectionProps = {
    db_type: DbType.Mssql,
    server: 'test-server',
    database: 'test-db',
    auth_type: AuthType.Sql,
    user: 'test-user',
  }

  it('renders correctly when show is true', () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: connectionProps,
        password: 'test-password',
      },
    })
    expect(wrapper.find('.modal-overlay').exists()).toBe(true)
    expect(wrapper.find('h3').text()).toBe('Save Connection')
  })

  it('does not render when show is false', () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: false,
        connection: connectionProps,
      },
    })
    expect(wrapper.find('.modal-overlay').exists()).toBe(false)
  })

  it('emits a close event when Cancel button is clicked', async () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: connectionProps,
      },
    })
    await wrapper.find('button[type="button"]').trigger('click')
    expect(wrapper.emitted().close).toBeTruthy()
  })

  it('emits a close event when overlay is clicked', async () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: connectionProps,
      },
    })
    await wrapper.find('.modal-overlay').trigger('click')
    expect(wrapper.emitted().close).toBeTruthy()
  })

  it('emits a save event with name when save button is clicked', async () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: connectionProps,
      },
    })

    await wrapper.find('input#connection-name').setValue('My Test Connection')
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted().save).toBeTruthy()
    expect(wrapper.emitted().save![0]).toEqual(['My Test Connection', undefined])
  })
  
  it('emits a save event with name and password when save button is clicked and checkbox is checked', async () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: { ...connectionProps, auth_type: AuthType.Sql },
        password: 'test-password',
      },
    })

    await wrapper.find('input#connection-name').setValue('My Test Connection')
    await wrapper.find('input[type="checkbox"]').setValue(true)
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted().save).toBeTruthy()
    expect(wrapper.emitted().save![0]).toEqual(['My Test Connection', 'test-password'])
  })

  it('shows an alert if connection name is empty', async () => {
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: connectionProps,
      },
    })

    await wrapper.find('form').trigger('submit.prevent')

    expect(alertSpy).toHaveBeenCalledWith('Please enter a name for the connection.')
    expect(wrapper.emitted().save).toBeFalsy()
  })

  it('shows save password checkbox only for SQL auth', async () => {
    const wrapper = mount(SaveConnectionDialog, {
      props: {
        show: true,
        connection: { ...connectionProps, auth_type: AuthType.Sql },
      },
    })
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(true)

    await wrapper.setProps({
      connection: { ...connectionProps, auth_type: AuthType.Integrated },
    })
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false)
  })
})
