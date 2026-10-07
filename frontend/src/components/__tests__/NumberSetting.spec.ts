import { describe, expect, it } from 'vitest'
import NumberSetting from '@/components/NumberSetting.vue'
import { mountWithPlugins } from './mount'

function mountBox(modelValue = 100) {
  return mountWithPlugins(NumberSetting, {
    props: { modelValue, label: 'Row limit', hint: 'From 1 to 1,000.' },
  })
}

describe('NumberSetting', () => {
  it('sends nothing while the user types', async () => {
    const wrapper = mountBox()
    await wrapper.find('input').setValue('1')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('sends the value on Enter and when the box loses the focus', async () => {
    const wrapper = mountBox()
    await wrapper.find('input').setValue('1500')
    await wrapper.find('input').trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([1500])

    await wrapper.find('input').setValue('42')
    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:focused', false)
    expect(wrapper.emitted('update:modelValue')?.[1]).toEqual([42])

    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:focused', true)
    expect(wrapper.emitted('update:modelValue')).toHaveLength(2)
  })

  it('puts the stored value back for an empty box or a text that is no number', async () => {
    const wrapper = mountBox()
    const input = wrapper.find('input')
    await input.setValue('')
    await input.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect((input.element as HTMLInputElement).value).toBe('100')
  })

  it('shows the value the parent keeps', async () => {
    const wrapper = mountBox()
    await wrapper.setProps({ modelValue: 7 })
    expect((wrapper.find('input').element as HTMLInputElement).value).toBe('7')
  })
})
