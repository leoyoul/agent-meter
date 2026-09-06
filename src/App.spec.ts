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
    expect(wrapper.text()).toContain('Agent Meter 设置')
    expect(wrapper.text()).toContain('菜单栏指标')
    expect(wrapper.text()).toContain('软件更新')
    expect(wrapper.text()).toContain('数据始终留在本机')
    wrapper.unmount()
  })

  it('provides accessible labels for icon controls', async () => {
    const wrapper = mount(App)
    expect(wrapper.get('button[aria-label="刷新数据"]').attributes('title')).toBe('刷新数据')
    expect(wrapper.get('button[aria-label="打开数据源设置"]').attributes('title')).toBe('数据源设置')
    wrapper.unmount()
  })

  it('toggles each compact menu metric independently', async () => {
    const wrapper = mount(App)
    await flushPromises()
    await wrapper.get('button[aria-label="打开数据源设置"]').trigger('click')
    const tokenSwitch = wrapper.get('button[aria-label="切换今日 Token 菜单栏指标"]')
    const ttftSwitch = wrapper.get('button[aria-label="切换首响时间菜单栏指标"]')
    const tpsSwitch = wrapper.get('button[aria-label="切换有效 TPS 菜单栏指标"]')
    expect(tokenSwitch.attributes('aria-checked')).toBe('true')
    expect(ttftSwitch.attributes('aria-checked')).toBe('false')
    expect(tpsSwitch.attributes('aria-checked')).toBe('false')
    await ttftSwitch.trigger('click')
    await vi.waitFor(() => expect(wrapper.get('button[aria-label="切换首响时间菜单栏指标"]').attributes('aria-checked')).toBe('true'))
    wrapper.unmount()
  })

  it('shows update status after a manual check', async () => {
    const wrapper = mount(App)
    await flushPromises()
    await wrapper.get('button[aria-label="打开数据源设置"]').trigger('click')
    const button = wrapper.findAll('button').find(item => item.text().includes('检查更新'))
    expect(button).toBeDefined()
    await button!.trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('已是最新版'))
    wrapper.unmount()
  })
})
