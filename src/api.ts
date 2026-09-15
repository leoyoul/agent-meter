import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import type { AnalyticsFilters, AppSettings, AppUpdateState, DashboardData, DataIntegrityStatus, ImportStatus, MetricSeriesPoint, MetricSummary, ModelEffortStat, PricingCatalogStatus, PricingModel, PricingRate, PricingRateInput, SourceInfo } from './shared'

export type MeterEvent = 'import-progress' | 'metrics-updated' | 'source-error' | 'settings-updated' | 'sources-updated' | 'update-requested'
const isTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
const iso = (minutes = 0) => new Date(Date.now() + minutes * 60_000).toISOString()
const tokenSet = (uncachedInput: number, cachedRead: number, cachedWrite: number, output: number, reasoning: number) => ({ uncachedInput, cachedRead, cachedWrite, output, reasoning, total: uncachedInput + cachedRead + cachedWrite + output })

const sources: SourceInfo[] = [
  { id: 1, name: 'Codex', sourceKind: 'codex_jsonl', dataCapability: 'metrics', rootPath: '~/.codex', enabled: true, available: true, fileCount: 627, totalBytes: 2_577_980_416, lastScanAt: iso(-2), error: null },
  { id: 2, name: 'ZCode', sourceKind: 'zcode_sqlite', dataCapability: 'metrics', rootPath: '~/.zcode/cli/db/db.sqlite', enabled: true, available: true, fileCount: 1, totalBytes: 8_912_896, lastScanAt: iso(-1), error: null },
  { id: 3, name: 'OpenCode', sourceKind: 'opencode_sqlite', dataCapability: 'metrics', rootPath: '~/.local/share/opencode/opencode.db', enabled: true, available: true, fileCount: 1, totalBytes: 29_360_128, lastScanAt: iso(-1), error: null },
  { id: 4, name: 'DSH', sourceKind: 'dsh_zstd', dataCapability: 'metrics', rootPath: '~/.dsh/sessions', enabled: true, available: true, fileCount: 37, totalBytes: 38_797_312, lastScanAt: iso(-1), error: null },
  { id: 5, name: 'Claude', sourceKind: 'ccswitch_sqlite', dataCapability: 'metrics', limitation: '仅统计经过 CC Switch 代理的 Claude Desktop 请求', rootPath: '~/.cc-switch/cc-switch.db', enabled: true, available: true, fileCount: 1, totalBytes: 0, lastScanAt: iso(-1), error: null },
  { id: 6, name: 'EvoX', sourceKind: 'evox_observability', dataCapability: 'metrics', rootPath: '~/.evox/agent/observability', enabled: true, available: true, fileCount: 12, totalBytes: 1_468_006, lastScanAt: iso(-1), error: null },
]
interface MockPerformanceSample { ttftMs: number | null; outputTokens: number; decodeMs: number | null }
type MockModelEffortStat = ModelEffortStat & { performanceSamples: MockPerformanceSample[] }
type MockModelDefinition = Omit<MockModelEffortStat, 'ttftSampleCount' | 'tpsSampleCount' | 'averageTtftMs' | 'averageEffectiveTps' | 'performanceSamples'> & { performanceSamples: MockPerformanceSample[] }
interface MockPerformanceSelection { ttft: Set<MockPerformanceSample>; tps: Set<MockPerformanceSample> }

const performanceSamples = (ttftCount: number, tpsCount: number, outputTokens: number, ttftMs: number, tps: number): MockPerformanceSample[] => Array.from({ length: Math.max(ttftCount, tpsCount) }, (_, index) => {
  const hasTps = index < tpsCount
  const output = hasTps ? Math.floor(outputTokens / tpsCount) + (index < outputTokens % tpsCount ? 1 : 0) : 0
  return { ttftMs: index < ttftCount ? ttftMs : null, outputTokens: output, decodeMs: hasTps ? output / tps * 1_000 : null }
})

const rawRows: MockModelDefinition[] = [
  { sourceId: 1, sourceName: 'Codex', provider: 'OpenAI', model: 'gpt-6-astra', reasoningEffort: 'medium', callCount: 63, performanceSamples: performanceSamples(9, 9, 84_600, 812, 17.4), tokens: tokenSet(146_200, 892_400, 0, 84_600, 31_200), estimatedCostNanoUsd: 6_584_000_000, pricing: { pricedObservations: 63, totalObservations: 63, pricedTokens: 1_123_200, totalTokens: 1_123_200, ratio: 1, complete: true } },
  { sourceId: 2, sourceName: 'ZCode', provider: 'omlx', model: 'Qwen3.8-27B-MLX-4bit', reasoningEffort: 'default', callCount: 31, performanceSamples: performanceSamples(25, 25, 61_800, 386, 29.8), tokens: tokenSet(97_800, 188_100, 12_400, 61_800, 18_500), estimatedCostNanoUsd: 0, pricing: { pricedObservations: 31, totalObservations: 31, pricedTokens: 360_100, totalTokens: 360_100, ratio: 1, complete: true } },
  { sourceId: 3, sourceName: 'OpenCode', provider: 'Meta', model: 'muse-spark-1.3-contributor-free', reasoningEffort: 'medium', callCount: 24, performanceSamples: performanceSamples(18, 18, 54_700, 640, 22.1), tokens: tokenSet(116_400, 421_300, 8_200, 54_700, 14_300), estimatedCostNanoUsd: 23_422_600, pricing: { pricedObservations: 24, totalObservations: 24, pricedTokens: 592_400, totalTokens: 600_600, ratio: .986, complete: false } },
]

const selectMockPerformance = (items: MockModelEffortStat[], period: AnalyticsFilters['period']): MockPerformanceSelection => {
  const performance = items.flatMap(row => row.performanceSamples)
  const ttft = performance.filter(sample => sample.ttftMs != null && sample.ttftMs >= 0)
  const tps = performance.filter(sample => sample.outputTokens > 0 && sample.decodeMs != null && sample.decodeMs >= 500)
  return { ttft: new Set(period === 'realtime' ? ttft.slice(-10) : ttft), tps: new Set(period === 'realtime' ? tps.slice(-10) : tps) }
}

const deriveMockPerformance = (definition: MockModelDefinition, selection?: MockPerformanceSelection): MockModelEffortStat => {
  const ttft = definition.performanceSamples.filter(sample => sample.ttftMs != null && sample.ttftMs >= 0 && (!selection || selection.ttft.has(sample)))
  const tps = definition.performanceSamples.filter(sample => sample.outputTokens > 0 && sample.decodeMs != null && sample.decodeMs >= 500 && (!selection || selection.tps.has(sample)))
  const totalOutput = tps.reduce((sum, sample) => sum + sample.outputTokens, 0)
  const totalDecodeMs = tps.reduce((sum, sample) => sum + (sample.decodeMs ?? 0), 0)
  return { ...definition, ttftSampleCount: ttft.length, tpsSampleCount: tps.length, averageTtftMs: ttft.length ? ttft.reduce((sum, sample) => sum + (sample.ttftMs ?? 0), 0) / ttft.length : null, averageEffectiveTps: totalOutput && totalDecodeMs ? totalOutput / (totalDecodeMs / 1_000) : null }
}

const rows: MockModelEffortStat[] = rawRows.map(row => deriveMockPerformance(row))
const series: MetricSeriesPoint[] = Array.from({ length: 10 }, (_, i) => ({ bucket: `call-${i + 1}`, label: `${String(9 + i).padStart(2, '0')}:20`, callCount: 1, ttftSampleCount: i === 2 ? 0 : 1, tpsSampleCount: i === 2 ? 0 : 1, averageTtftMs: i === 2 ? null : 490 + i * 32, averageEffectiveTps: i === 2 ? null : 15.2 + (i % 4) * 2.3, totalTokens: 44_000 + i * 13_700, estimatedCostNanoUsd: 52_000_000 + i * 8_900_000 }))
const scopedRows = (items: MockModelEffortStat[], period: AnalyticsFilters['period']): MockModelEffortStat[] => {
  const selection = selectMockPerformance(items, period)
  return items.map(row => deriveMockPerformance(row, selection))
}

const summarize = (items: MockModelEffortStat[], period: AnalyticsFilters['period'] = 'realtime'): MetricSummary => {
  const tokens = items.reduce((a, row) => ({ uncachedInput: a.uncachedInput + row.tokens.uncachedInput, cachedRead: a.cachedRead + row.tokens.cachedRead, cachedWrite: a.cachedWrite + row.tokens.cachedWrite, output: a.output + row.tokens.output, reasoning: a.reasoning + row.tokens.reasoning, total: a.total + row.tokens.total }), tokenSet(0, 0, 0, 0, 0))
  const pricedTokens = items.reduce((sum, row) => sum + row.pricing.pricedTokens, 0)
  const selection = selectMockPerformance(items, period)
  const ttft = [...selection.ttft]
  const tps = [...selection.tps]
  const totalOutput = tps.reduce((sum, sample) => sum + sample.outputTokens, 0)
  const totalDecodeMs = tps.reduce((sum, sample) => sum + (sample.decodeMs ?? 0), 0)
  return { period: 'realtime', callCount: items.reduce((sum, row) => sum + row.callCount, 0), ttftSampleCount: ttft.length, tpsSampleCount: tps.length, averageTtftMs: ttft.length ? ttft.reduce((sum, sample) => sum + (sample.ttftMs ?? 0), 0) / ttft.length : null, averageEffectiveTps: totalOutput && totalDecodeMs ? totalOutput / (totalDecodeMs / 1_000) : null, tokens, estimatedCostNanoUsd: items.reduce((sum, row) => sum + row.estimatedCostNanoUsd, 0), pricing: { pricedObservations: items.reduce((sum, row) => sum + row.pricing.pricedObservations, 0), totalObservations: items.reduce((sum, row) => sum + row.pricing.totalObservations, 0), pricedTokens, totalTokens: tokens.total, ratio: tokens.total ? pricedTokens / tokens.total : 1, complete: pricedTokens === tokens.total }, lastUpdatedAt: iso(-1) }
}
let mockStatus: ImportStatus = { running: false, paused: false, filesDone: 627, filesTotal: 627, bytesDone: 2_656_518_758, bytesTotal: 2_656_518_758, currentFile: null, startedAt: iso(-48), message: '本机指标均已同步' }
let mockAutostart = false
let mockSettings: AppSettings = { menuMetrics: { todayTokens: true, ttft: false, effectiveTps: false, estimatedCost: false }, showAppIcon: false, menuPeriod: 'realtime', activeSourceKind: null, updates: { automaticCheck: true, lastCheckedAt: null } }
let pendingUpdate: Update | null = null
let nextRateId = 3
let pricingRates: PricingRate[] = [
  { id: 1, vendor: 'OpenAI', model: 'gpt-6-astra', aliases: [], currency: 'USD', inputUsdPerMillion: '10', cachedReadUsdPerMillion: '1', cachedWriteUsdPerMillion: '12.5', outputUsdPerMillion: '50', effectiveFrom: '2026-09-07', effectiveTo: null, sourceUrl: 'https://developers.openai.com/api/docs/models/gpt-6-astra', verifiedAt: '2026-09-07', origin: 'builtin', callCount: 63 },
  { id: 2, vendor: 'Meta', model: 'muse-spark-1.3', aliases: ['muse-spark-1.3-contributor', 'muse-spark-1.3-contributor-free'], currency: 'USD', inputUsdPerMillion: '0.1', cachedReadUsdPerMillion: '0.002', cachedWriteUsdPerMillion: null, outputUsdPerMillion: '0.2', effectiveFrom: '2026-09-03', effectiveTo: null, sourceUrl: 'https://vercel.com/changelog/muse-spark-1-3-now-available-on-ai-gateway', verifiedAt: '2026-09-06', origin: 'builtin', callCount: 24 },
]
const pricingStatus = (): PricingCatalogStatus => ({ version: '2026-09-07.1', currency: 'USD', verifiedAt: '2026-09-07', pricedObservations: 118, totalObservations: 118, rates: structuredClone(pricingRates) })
const pricingModels = (): PricingModel[] => [
  { vendor: 'OpenAI', model: 'gpt-6-astra', callCount: 63, totalTokens: 1_123_200, pricingStatus: 'priced', rates: pricingRates.filter(rate => rate.model === 'gpt-6-astra') },
  { vendor: 'Meta', model: 'muse-spark-1.3', callCount: 24, totalTokens: 600_600, pricingStatus: 'partial', rates: pricingRates.filter(rate => rate.model === 'muse-spark-1.3') },
  { vendor: 'unknown', model: 'unpriced-model', callCount: 8, totalTokens: 88_000, pricingStatus: 'unpriced', rates: [] },
]
const integrity: DataIntegrityStatus = { reconciled: true, sources: sources.filter(source => source.dataCapability === 'metrics').map(source => ({ sourceId: source.id, sourceName: source.name, rawCallCount: source.id === 1 ? 463 : source.id === 5 ? 42 : 24, indexedCallCount: source.id === 1 ? 463 : source.id === 5 ? 42 : 24, difference: 0, unreadBytes: 0, parseErrorCount: 0, lastCallAt: iso(-1), syncDelayMs: 60_000, reconciled: true, error: null })) }
const call = async <T>(command: string, args?: Record<string, unknown>, fallback?: () => T): Promise<T> => { if (isTauri()) return invoke<T>(command, args); await new Promise(resolve => setTimeout(resolve, 20)); if (!fallback) throw new Error(`Mock not implemented: ${command}`); return fallback() }
const filtered = (filters: AnalyticsFilters) => rows.filter(row => (!filters.sourceId || row.sourceId === filters.sourceId) && (!filters.model || row.model === filters.model) && (!filters.reasoningEffort || row.reasoningEffort === filters.reasoningEffort))

export const meterApi = {
  isTauri,
  discoverSources: () => call<SourceInfo[]>('discover_sources', undefined, () => structuredClone(sources)),
  startImport: (force = false) => call<ImportStatus>('start_import', { force }, () => (mockStatus = { ...mockStatus, running: true, paused: false, message: force ? '正在重建索引' : '正在同步本机指标' })),
  pauseImport: () => call<ImportStatus>('pause_import', undefined, () => (mockStatus = { ...mockStatus, running: false, paused: true, message: '导入已暂停' })),
  getImportStatus: () => call<ImportStatus>('get_import_status', undefined, () => ({ ...mockStatus })),
  queryDashboard: (filters: AnalyticsFilters) => call<DashboardData>('query_dashboard', { filters }, () => { const items = scopedRows(filtered(filters), filters.period); return { summary: { ...summarize(items, filters.period), period: filters.period }, series: structuredClone(series), stats: structuredClone(items), importStatus: { ...mockStatus }, integrity: structuredClone(integrity) } }),
  queryMetricSummary: (filters: AnalyticsFilters) => call<MetricSummary>('query_metric_summary', { filters }, () => ({ ...summarize(filtered(filters), filters.period), period: filters.period })),
  queryMetricSeries: (filters: AnalyticsFilters) => call<MetricSeriesPoint[]>('query_metric_series', { filters }, () => structuredClone(series)),
  queryModelEffortStats: (filters: AnalyticsFilters) => call<ModelEffortStat[]>('query_model_effort_stats', { filters }, () => structuredClone(scopedRows(filtered(filters), filters.period))),
  getDataIntegrityStatus: () => call<DataIntegrityStatus>('get_data_integrity_status', undefined, () => structuredClone(integrity)),
  getPricingCatalogStatus: () => call<PricingCatalogStatus>('get_pricing_catalog_status', undefined, pricingStatus),
  listPricingModels: () => call<PricingModel[]>('list_pricing_models', undefined, () => structuredClone(pricingModels())),
  createPricingRate: (input: PricingRateInput) => call<PricingRate>('create_pricing_rate', { input }, () => { const rate: PricingRate = { ...input, id: nextRateId++, currency: 'USD', verifiedAt: new Date().toISOString().slice(0, 10), origin: 'custom', callCount: 0 }; pricingRates.push(rate); return structuredClone(rate) }),
  updatePricingRate: (id: number, input: PricingRateInput) => call<PricingRate>('update_pricing_rate', { id, input }, () => { const index = pricingRates.findIndex(rate => rate.id === id); const rate: PricingRate = { ...pricingRates[index], ...input, origin: 'custom' }; pricingRates[index] = rate; return structuredClone(rate) }),
  deletePricingRate: (id: number) => call<void>('delete_pricing_rate', { id }, () => { pricingRates = pricingRates.filter(rate => rate.id !== id) }),
  repriceUsage: () => call<PricingCatalogStatus>('reprice_usage', undefined, pricingStatus),
  updateSource: (id: number, enabled: boolean) => call<SourceInfo>('update_source', { sourceId: id, enabled }, () => { const source = sources.find(item => item.id === id)!; source.enabled = enabled; return { ...source } }),
  getAppSettings: () => call<AppSettings>('get_app_settings', undefined, () => structuredClone(mockSettings)),
  updateAppSettings: (settings: AppSettings) => call<AppSettings>('update_app_settings', { settings }, () => (mockSettings = structuredClone(settings))),
  getCurrentVersion: async () => isTauri() ? getVersion() : '0.4.5',
  checkForUpdate: async (): Promise<AppUpdateState> => { const currentVersion = await meterApi.getCurrentVersion(); if (!isTauri()) return { phase: 'current', currentVersion, downloadedBytes: 0 }; if (pendingUpdate) { await pendingUpdate.close(); pendingUpdate = null }; const update = await check({ timeout: 20_000 }); if (!update) return { phase: 'current', currentVersion, downloadedBytes: 0 }; pendingUpdate = update; return { phase: 'available', currentVersion, version: update.version, notes: update.body, downloadedBytes: 0 } },
  downloadAndInstallUpdate: async (onState: (state: AppUpdateState) => void) => { const currentVersion = await meterApi.getCurrentVersion(); if (!pendingUpdate) throw new Error('没有可安装的更新，请先检查更新'); const update = pendingUpdate; let downloadedBytes = 0; let totalBytes: number | undefined; await update.download(event => { if (event.event === 'Started') totalBytes = event.data.contentLength; if (event.event === 'Progress') downloadedBytes += event.data.chunkLength; onState({ phase: 'downloading', currentVersion, version: update.version, notes: update.body, downloadedBytes, totalBytes }) }, { timeout: 120_000 }); await meterApi.pauseImport(); await update.install(); onState({ phase: 'ready', currentVersion, version: update.version, downloadedBytes, totalBytes }); await relaunch() },
  isAutostartEnabled: async () => isTauri() ? isEnabled() : mockAutostart,
  setAutostartEnabled: async (enabledValue: boolean) => { if (isTauri()) enabledValue ? await enable() : await disable(); else mockAutostart = enabledValue; return enabledValue },
  showDashboardWindow: () => call<void>('show_dashboard_window', undefined, () => undefined),
  showSettingsWindow: () => call<void>('show_settings_window', undefined, () => undefined),
  on: async <T>(event: MeterEvent, handler: (payload: T) => void): Promise<UnlistenFn> => isTauri() ? listen<T>(event, ({ payload }) => handler(payload)) : () => undefined,
}
