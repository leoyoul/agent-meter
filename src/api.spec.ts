import { describe, expect, it, vi } from 'vitest'
import { meterApi } from './api'

const invokeMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

describe('browser mock API v0.4', () => {
  it('returns the four-metric contract', async () => {
    const filters = { period: 'realtime' as const }
    const [summary, series, matrix] = await Promise.all([meterApi.queryMetricSummary(filters), meterApi.queryMetricSeries(filters), meterApi.queryModelEffortStats(filters)])
    expect(summary.tokens.total).toBeGreaterThan(0)
    expect(summary.averageTtftMs).toBeGreaterThan(0)
    expect(summary.averageEffectiveTps).toBeGreaterThan(0)
    expect(summary.estimatedCostNanoUsd).toBeGreaterThan(0)
    expect(summary.callCount).toBeGreaterThan(summary.performanceSampleCount)
    expect(series).toHaveLength(10)
    expect(new Set(matrix.map(row => row.sourceName))).toEqual(new Set(['Codex', 'ZCode', 'OpenCode']))
  })

  it('returns the dashboard metrics in one aggregate payload', async () => {
    const dashboard = await meterApi.queryDashboard({ period: 'realtime' })
    expect(dashboard.summary.tokens.total).toBeGreaterThan(0)
    expect(dashboard.series).toHaveLength(10)
    expect(dashboard.stats).toHaveLength(3)
    expect(dashboard.importStatus.filesTotal).toBeGreaterThan(0)
    expect(dashboard.integrity.reconciled).toBe(true)
  })

  it('filters by source, model and effort', async () => {
    const rows = await meterApi.queryModelEffortStats({ period: 'today', sourceId: 2, model: 'Qwen3.8-27B-MLX-4bit', reasoningEffort: 'default' })
    expect(rows).toHaveLength(1)
    expect(rows[0].sourceName).toBe('ZCode')
    expect(rows[0].estimatedCostNanoUsd).toBe(0)
    expect(rows[0].pricing.complete).toBe(true)
  })

  it('exposes versioned official pricing metadata', async () => {
    const catalog = await meterApi.getPricingCatalogStatus()
    expect(catalog.version).toMatch(/^\d{4}-\d{2}-\d{2}/)
    expect(catalog.rates.every(rate => rate.sourceUrl.startsWith('https://'))).toBe(true)
  })

  it('supports pricing CRUD and data integrity status', async () => {
    const before = await meterApi.listPricingModels()
    expect(before.some(model => model.model === 'unpriced-model')).toBe(true)
    const created = await meterApi.createPricingRate({ vendor: 'Test', model: 'new-model', aliases: [], inputUsdPerMillion: '1', cachedReadUsdPerMillion: null, cachedWriteUsdPerMillion: '0', outputUsdPerMillion: '2', effectiveFrom: '2026-09-07', effectiveTo: null, sourceUrl: 'https://example.com' })
    expect(created.cachedReadUsdPerMillion).toBeNull()
    expect(created.cachedWriteUsdPerMillion).toBe('0')
    await meterApi.deletePricingRate(created.id)
    expect((await meterApi.getDataIntegrityStatus()).reconciled).toBe(true)
  })

  it('persists fee visibility and shared period', async () => {
    const settings = await meterApi.getAppSettings()
    settings.menuMetrics.estimatedCost = true
    settings.menuPeriod = 'year'
    settings.activeSourceKind = 'dsh_zstd'
    const updated = await meterApi.updateAppSettings(settings)
    expect(updated.menuMetrics.estimatedCost).toBe(true)
    expect(updated.menuPeriod).toBe('year')
    expect(updated.activeSourceKind).toBe('dsh_zstd')
  })

  it('discovers all sources and exposes Claude as detection-only', async () => {
    const discovered = await meterApi.discoverSources()
    expect(discovered.map(source => source.name)).toEqual(['Codex', 'ZCode', 'OpenCode', 'DSH', 'Claude', 'EvoX'])
    expect(discovered.find(source => source.name === 'Claude')).toMatchObject({ dataCapability: 'noUsageLog', limitation: '未发现可统计的本地 Token 记录' })
  })

  it('supports source and import lifecycle controls', async () => {
    const [source] = await meterApi.discoverSources()
    expect((await meterApi.updateSource(source.id, false)).enabled).toBe(false)
    await meterApi.updateSource(source.id, true)
    expect((await meterApi.startImport(true)).message).toContain('重建')
    expect((await meterApi.pauseImport()).paused).toBe(true)
  })

  it('uses the Tauri camelCase sourceId argument', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    invokeMock.mockResolvedValue({ id: 7, enabled: false })

    await meterApi.updateSource(7, false)

    expect(invokeMock).toHaveBeenCalledWith('update_source', { sourceId: 7, enabled: false })
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  })
})
