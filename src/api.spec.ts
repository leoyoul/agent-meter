import { describe, expect, it } from 'vitest'
import { meterApi } from './api'

describe('browser mock API v0.4', () => {
  it('returns the four-metric contract', async () => {
    const filters = { period: 'realtime' as const }
    const [summary, series, matrix] = await Promise.all([meterApi.queryMetricSummary(filters), meterApi.queryMetricSeries(filters), meterApi.queryModelEffortStats(filters)])
    expect(summary.tokens.total).toBeGreaterThan(0)
    expect(summary.averageTtftMs).toBeGreaterThan(0)
    expect(summary.averageEffectiveTps).toBeGreaterThan(0)
    expect(summary.estimatedCostNanoUsd).toBeGreaterThan(0)
    expect(series).toHaveLength(10)
    expect(new Set(matrix.map(row => row.sourceName))).toEqual(new Set(['Codex', 'ZCode', 'OpenCode', 'DSH', 'EvoX']))
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
})
