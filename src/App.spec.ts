import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App.vue'
import { meterApi } from './api'

describe('Agent Meter v0.4 windows', () => {
  beforeEach(async () => {
    window.history.replaceState({}, '', '/')
    const settings = await meterApi.getAppSettings()
    settings.activeSourceKind = null
    settings.menuPeriod = 'realtime'
    settings.menuMetrics = { todayTokens: true, ttft: false, effectiveTps: false, estimatedCost: false }
    await meterApi.updateAppSettings(settings)
  })

  afterEach(() => {
    document.body.replaceChildren()
    window.history.replaceState({}, '', '/')
  })

  it('renders seven top-level source tabs and the four-metric matrix', async () => {
    const wrapper = mount(App, { attachTo: document.body })
    await vi.waitFor(() => expect(wrapper.findAll('.source-tab')).toHaveLength(7))
    expect(wrapper.findAll('.source-tab').map(tab => tab.text())).toEqual(['全部', 'Codex', 'ZCode', 'OpenCode', 'DSH', 'Claude', 'EvoX'])
    expect(wrapper.find('select option').text()).toBe('全部模型')
    expect(wrapper.findAll('.view-tabs button').map(button => button.text())).toEqual(['分析', '价格'])
    expect(wrapper.text()).toContain('来源 × 模型 × 推理强度')
    expect(wrapper.text()).not.toContain('主代理与子代理')
    expect(wrapper.text()).not.toContain('任务明细')
    wrapper.unmount()
  })

  it('lists used models and opens the pricing editor', async () => {
    const wrapper = mount(App)
    await vi.waitFor(() => expect(wrapper.findAll('.view-tabs button')).toHaveLength(2))
    await wrapper.findAll('.view-tabs button')[1].trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('gpt-6-astra'))
    expect(wrapper.text()).toContain('unpriced-model')
    const add = wrapper.findAll('button').find(button => button.text().includes('新增价格'))!
    await add.trigger('click')
    expect(wrapper.find('.price-editor').exists()).toBe(true)
    expect(wrapper.text()).toContain('空白表示该桶未计价')
    wrapper.unmount()
  })

  it('persists source selection and clears unavailable Claude metrics', async () => {
    const wrapper = mount(App)
    await vi.waitFor(() => expect(wrapper.findAll('.source-tab')).toHaveLength(7))
    await wrapper.findAll('.source-tab')[4].trigger('click')
    await vi.waitFor(async () => expect((await meterApi.getAppSettings()).activeSourceKind).toBe('dsh_zstd'))
    expect(wrapper.findAll('.source-tab')[4].attributes('aria-selected')).toBe('true')
    await wrapper.findAll('.source-tab')[5].trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('未发现可统计的本地 Token 记录'))
    expect(wrapper.find('.kpi-grid').exists()).toBe(false)
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

  it('renders settings as a standalone page and toggles menu metrics', async () => {
    window.history.replaceState({}, '', '/?view=settings')
    const wrapper = mount(App)
    await vi.waitFor(() => expect(wrapper.text()).toContain('菜单栏、数据源与应用更新'))
    expect(wrapper.find('.app-shell').exists()).toBe(false)
    for (const label of ['速', '首', '费']) {
      const selector = `button[aria-label="切换${label}菜单栏指标"]`
      expect(wrapper.get(selector).attributes('aria-checked')).toBe('false')
      await wrapper.get(selector).trigger('click')
      await vi.waitFor(() => expect(wrapper.get(selector).attributes('aria-checked')).toBe('true'))
    }
    const iconSwitch = 'button[aria-label="切换应用图标显示"]'
    expect(wrapper.get(iconSwitch).attributes('aria-checked')).toBe('false')
    await wrapper.get(iconSwitch).trigger('click')
    await vi.waitFor(() => expect(wrapper.get(iconSwitch).attributes('aria-checked')).toBe('true'))
    expect(wrapper.text()).toContain('未发现可统计的本地 Token 记录')
    expect(wrapper.text()).toContain('数据只留在本机')
    wrapper.unmount()
  })

  it('keeps update checks in the standalone settings page', async () => {
    window.history.replaceState({}, '', '/?view=settings')
    const wrapper = mount(App)
    await vi.waitFor(() => expect(wrapper.text()).toContain('API 等价价目'))
    const button = wrapper.findAll('button').find(item => item.text().includes('检查更新'))!
    await button.trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('已是最新版'))
    wrapper.unmount()
  })
})
