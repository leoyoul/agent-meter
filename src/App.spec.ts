import { mount, flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App.vue'

describe('Agent Meter dashboard', () => {
  afterEach(() => document.body.replaceChildren())

  it('renders metric sections and opens data settings', async () => {
    const wrapper = mount(App, { attachTo: document.body })
    await vi.waitFor(() => {
      expect(wrapper.findAll('tbody tr').length).toBeGreaterThanOrEqual(3)
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Agent Meter')
    expect(wrapper.text()).toContain('近 5 轮有效 TPS')
    expect(wrapper.text()).toContain('模型表现')
    await wrapper.get('button[aria-label="打开数据源设置"]').trigger('click')
    expect(wrapper.text()).toContain('数据与索引')
    expect(wrapper.text()).toContain('数据始终留在本机')
    wrapper.unmount()
  })

  it('provides accessible labels for icon controls', async () => {
    const wrapper = mount(App)
    expect(wrapper.get('button[aria-label="刷新数据"]').attributes('title')).toBe('刷新数据')
    expect(wrapper.get('button[aria-label="打开数据源设置"]').attributes('title')).toBe('数据源设置')
    wrapper.unmount()
  })
})
