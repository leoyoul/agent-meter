import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App.vue'

describe('Agent Meter v0.3 dashboard', () => {
  afterEach(() => document.body.replaceChildren())

  it('renders four metrics, periods, three-source matrix and no task detail sections', async () => {
    const wrapper = mount(App, { attachTo: document.body })
    await vi.waitFor(() => expect(wrapper.findAll('tbody tr').length).toBeGreaterThanOrEqual(4))
    expect(wrapper.text()).toContain('平均有效 TPS')
    expect(wrapper.text()).toContain('API 等价费用')
    expect(wrapper.text()).toContain('来源 × 模型 × 推理强度')
    expect(wrapper.text()).toContain('Codex')
    expect(wrapper.text()).toContain('ZCode')
    expect(wrapper.text()).toContain('OpenCode')
    expect(wrapper.text()).not.toContain('主代理与子代理')
    expect(wrapper.text()).not.toContain('任务明细')
    expect(wrapper.findAll('.period-control button').map(button => button.text())).toEqual(['实时', '今日', '本周', '本月', '本年'])
    wrapper.unmount()
  })

  it('switches the shared metric period', async () => {
    const wrapper = mount(App)
    await flushPromises()
    await wrapper.findAll('.period-control button')[2].trigger('click')
    await vi.waitFor(() => expect(wrapper.findAll('.period-control button')[2].classes()).toContain('active'))
    expect(wrapper.text()).toContain('本周 Token 累计')
    wrapper.unmount()
  })

  it('toggles all four fixed menu metrics independently', async () => {
    const wrapper = mount(App)
    await flushPromises()
    await wrapper.get('button[aria-label="打开数据源设置"]').trigger('click')
    for (const label of ['速', '首', '费']) {
      const selector = `button[aria-label="切换${label}菜单栏指标"]`
      expect(wrapper.get(selector).attributes('aria-checked')).toBe('false')
      await wrapper.get(selector).trigger('click')
      await vi.waitFor(() => expect(wrapper.get(selector).attributes('aria-checked')).toBe('true'))
    }
    expect(wrapper.get('button[aria-label="切换量菜单栏指标"]').attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })

  it('shows missing timing and partial pricing honestly', async () => {
    const wrapper = mount(App)
    await vi.waitFor(() => expect(wrapper.text()).toContain('muse-pro'))
    const muse = wrapper.findAll('tbody tr').find(row => row.text().includes('muse-pro'))!
    expect(muse.text()).toContain('—')
    expect(muse.text()).toContain('$0.00+')
    expect(muse.text()).toContain('0% 覆盖')
    wrapper.unmount()
  })

  it('keeps updater and privacy controls in settings', async () => {
    const wrapper = mount(App)
    await flushPromises()
    await wrapper.get('button[aria-label="打开数据源设置"]').trigger('click')
    expect(wrapper.text()).toContain('API 等价价目')
    expect(wrapper.text()).toContain('数据只留在本机')
    const button = wrapper.findAll('button').find(item => item.text().includes('检查更新'))!
    await button.trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('已是最新版'))
    wrapper.unmount()
  })
})
