import { afterEach, describe, expect, it, vi } from 'vitest'
import { meterApi } from './api'

const invokeMock = vi.hoisted(() => vi.fn())
const checkMock = vi.hoisted(() => vi.fn())
const getVersionMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: getVersionMock }))
vi.mock('@tauri-apps/plugin-updater', () => ({ check: checkMock }))

describe('browser mock API v0.4', () => {
  afterEach(() => {
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
    invokeMock.mockReset()
    checkMock.mockReset()
    getVersionMock.mockReset()
  })

  it('returns the four-metric contract', async () => {
    const filters = { period: 'realtime' as const }
    const [summary, series, matrix] = await Promise.all([meterApi.queryMetricSummary(filters), meterApi.queryMetricSeries(filters), meterApi.queryModelEffortStats(filters)])
    expect(summary.tokens.total).toBeGreaterThan(0)
    expect(summary.averageTtftMs).toBeGreaterThan(0)
    expect(summary.averageEffectiveTps).toBeGreaterThan(0)
    expect(summary.estimatedCostNanoUsd).toBeGreaterThan(0)
    expect(summary.callCount).toBeGreaterThan(summary.ttftSampleCount)
    expect(summary.callCount).toBeGreaterThan(summary.tpsSampleCount)
    expect(summary.ttftSampleCount).toBe(10)
    expect(summary.tpsSampleCount).toBe(10)
    expect(summary.averageTtftMs).toBe(640)
    expect(summary.averageEffectiveTps).toBe(22.1)
    expect(series).toHaveLength(10)
    expect(new Set(matrix.map(row => row.sourceName))).toEqual(new Set(['Codex', 'ZCode', 'OpenCode']))
  })

  it('aggregates all period samples instead of averaging model averages', async () => {
    const summary = await meterApi.queryMetricSummary({ period: 'today' })
    expect(summary.ttftSampleCount).toBe(52)
    expect(summary.tpsSampleCount).toBe(52)
    expect(summary.averageTtftMs).toBeCloseTo((9 * 812 + 25 * 386 + 18 * 640) / 52, 5)
    expect(summary.averageEffectiveTps).toBeCloseTo((84_600 + 61_800 + 54_700) / (84_600 / 17.4 + 61_800 / 29.8 + 54_700 / 22.1), 5)
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

  it('does not claim that a browser preview is up to date', async () => {
    const state = await meterApi.checkForUpdate()

    expect(state.phase).toBe('unavailable')
    expect(state.platform).toBe('browser')
    expect(state.message).toContain('本地预览不支持')
    expect(state.currentVersion).toBe('0.4.8')
  })

  it('reports an available update for the current Tauri target', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    getVersionMock.mockResolvedValue('0.4.5')
    invokeMock.mockImplementation(async (command: string) => command === 'get_update_target' ? 'darwin-aarch64' : undefined)
    checkMock.mockResolvedValue({ version: '0.4.8', body: '更新说明', close: vi.fn() })

    const state = await meterApi.checkForUpdate()

    expect(state).toMatchObject({ phase: 'available', currentVersion: '0.4.5', platform: 'darwin-aarch64', version: '0.4.8' })
    expect(checkMock).toHaveBeenCalledWith({ timeout: 20_000 })
  })

  it('reports a current version only after the updater confirms no newer release', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    getVersionMock.mockResolvedValue('0.4.8')
    invokeMock.mockResolvedValue('darwin-aarch64')
    checkMock.mockResolvedValue(null)

    const state = await meterApi.checkForUpdate()

    expect(state).toMatchObject({ phase: 'current', currentVersion: '0.4.8', platform: 'darwin-aarch64' })
    expect(state.message).toContain('没有高于当前版本')
  })

  it('distinguishes an unavailable update target from a network failure', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    getVersionMock.mockResolvedValue('0.4.5')
    invokeMock.mockResolvedValue('darwin-aarch64')
    checkMock.mockRejectedValueOnce(new Error('None of the fallback platforms [darwin-aarch64-app, darwin-aarch64] were found'))

    const unavailable = await meterApi.checkForUpdate()
    expect(unavailable).toMatchObject({ phase: 'unavailable', platform: 'darwin-aarch64', currentVersion: '0.4.5' })
    expect(unavailable.message).toContain('没有找到适配')

    checkMock.mockRejectedValueOnce(new Error('network timeout'))
    await expect(meterApi.checkForUpdate()).resolves.toMatchObject({ phase: 'error', platform: 'darwin-aarch64', currentVersion: '0.4.5', error: 'network timeout' })
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

  it('discovers all sources and exposes Claude proxy metrics', async () => {
    const discovered = await meterApi.discoverSources()
    expect(discovered.map(source => source.name)).toEqual(['Codex', 'ZCode', 'OpenCode', 'DSH', 'Claude', 'EvoX'])
    expect(discovered.find(source => source.name === 'Claude')).toMatchObject({ dataCapability: 'metrics', sourceKind: 'ccswitch_sqlite', rootPath: '~/.cc-switch/cc-switch.db' })
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
