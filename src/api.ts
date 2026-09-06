import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import type { AnalyticsFilters, AppSettings, AppUpdateState, ImportStatus, MetricSeriesPoint, MetricSummary, ModelEffortStat, PricingCatalogStatus, SourceInfo } from './shared'

export type MeterEvent = 'import-progress' | 'metrics-updated' | 'source-error'
const isTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
const iso = (minutes = 0) => new Date(Date.now() + minutes * 60_000).toISOString()
const tokenSet = (uncachedInput: number, cachedRead: number, cachedWrite: number, output: number, reasoning: number) => ({ uncachedInput, cachedRead, cachedWrite, output, reasoning, total: uncachedInput + cachedRead + cachedWrite + output })

const sources: SourceInfo[] = [
  { id: 1, name: 'Codex', rootPath: '~/.codex', enabled: true, available: true, fileCount: 546, totalBytes: 2_577_980_416, lastScanAt: iso(-2), error: null },
  { id: 2, name: 'ZCode', rootPath: '~/.zcode/cli/db/db.sqlite', enabled: true, available: true, fileCount: 1, totalBytes: 8_912_896, lastScanAt: iso(-1), error: null },
  { id: 3, name: 'OpenCode', rootPath: '~/.local/share/opencode/opencode.db', enabled: true, available: true, fileCount: 1, totalBytes: 29_360_128, lastScanAt: iso(-1), error: null },
]
const matrix: ModelEffortStat[] = [
  { sourceId: 1, sourceName: 'Codex', provider: 'OpenAI', model: 'gpt-5.6-sol', reasoningEffort: 'high', observationCount: 42, averageTtftMs: 812, averageEffectiveTps: 17.4, tokens: tokenSet(146_200, 892_400, 0, 84_600, 31_200), estimatedCostNanoUsd: 2_636_160_000, pricing: { pricedObservations: 42, totalObservations: 42, pricedTokens: 1_123_200, totalTokens: 1_123_200, ratio: 1, complete: true } },
  { sourceId: 2, sourceName: 'ZCode', provider: 'omlx', model: 'Qwen3.8-27B-MLX-4bit', reasoningEffort: 'default', observationCount: 31, averageTtftMs: 386, averageEffectiveTps: 29.8, tokens: tokenSet(97_800, 188_100, 12_400, 61_800, 18_500), estimatedCostNanoUsd: 0, pricing: { pricedObservations: 31, totalObservations: 31, pricedTokens: 360_100, totalTokens: 360_100, ratio: 1, complete: true } },
  { sourceId: 3, sourceName: 'OpenCode', provider: 'MiniMax', model: 'MiniMax-M2.7', reasoningEffort: 'medium', observationCount: 24, averageTtftMs: 640, averageEffectiveTps: 22.1, tokens: tokenSet(116_400, 421_300, 8_200, 54_700, 14_300), estimatedCostNanoUsd: 136_642_500, pricing: { pricedObservations: 24, totalObservations: 24, pricedTokens: 600_600, totalTokens: 600_600, ratio: 1, complete: true } },
  { sourceId: 3, sourceName: 'OpenCode', provider: 'custom', model: 'muse-pro', reasoningEffort: 'unknown', observationCount: 9, averageTtftMs: null, averageEffectiveTps: null, tokens: tokenSet(32_100, 0, 0, 11_900, 2_600), estimatedCostNanoUsd: 0, pricing: { pricedObservations: 0, totalObservations: 9, pricedTokens: 0, totalTokens: 44_000, ratio: 0, complete: false } },
]
const series: MetricSeriesPoint[] = Array.from({ length: 10 }, (_, i) => ({ bucket: `call-${i + 1}`, label: `${String(9 + i).padStart(2, '0')}:20`, observationCount: 1, averageTtftMs: i === 2 ? null : 490 + i * 32, averageEffectiveTps: i === 2 ? null : 15.2 + (i % 4) * 2.3, totalTokens: 44_000 + i * 13_700, estimatedCostNanoUsd: i === 2 ? 0 : 52_000_000 + i * 8_900_000 }))
const summarize = (rows: ModelEffortStat[]): MetricSummary => {
  const tokens = rows.reduce((a, row) => ({ uncachedInput: a.uncachedInput + row.tokens.uncachedInput, cachedRead: a.cachedRead + row.tokens.cachedRead, cachedWrite: a.cachedWrite + row.tokens.cachedWrite, output: a.output + row.tokens.output, reasoning: a.reasoning + row.tokens.reasoning, total: a.total + row.tokens.total }), tokenSet(0, 0, 0, 0, 0))
  const pricedTokens = rows.reduce((sum, row) => sum + row.pricing.pricedTokens, 0)
  const ttft = rows.flatMap(row => row.averageTtftMs == null ? [] : [row.averageTtftMs])
  const tps = rows.flatMap(row => row.averageEffectiveTps == null ? [] : [row.averageEffectiveTps])
  return { period: 'realtime', observationCount: rows.reduce((sum, row) => sum + row.observationCount, 0), averageTtftMs: ttft.length ? ttft.reduce((a, b) => a + b, 0) / ttft.length : null, averageEffectiveTps: tps.length ? tps.reduce((a, b) => a + b, 0) / tps.length : null, tokens, estimatedCostNanoUsd: rows.reduce((sum, row) => sum + row.estimatedCostNanoUsd, 0), pricing: { pricedObservations: rows.reduce((sum, row) => sum + row.pricing.pricedObservations, 0), totalObservations: rows.reduce((sum, row) => sum + row.pricing.totalObservations, 0), pricedTokens, totalTokens: tokens.total, ratio: tokens.total ? pricedTokens / tokens.total : 1, complete: pricedTokens === tokens.total }, lastUpdatedAt: iso(-1) }
}
let mockStatus: ImportStatus = { running: false, paused: false, filesDone: 548, filesTotal: 548, bytesDone: 2_616_253_440, bytesTotal: 2_616_253_440, currentFile: null, startedAt: iso(-48), message: '三个数据源均已同步' }
let mockAutostart = false
let mockSettings: AppSettings = { menuMetrics: { todayTokens: true, ttft: false, effectiveTps: false, estimatedCost: false }, menuPeriod: 'realtime', updates: { automaticCheck: true, lastCheckedAt: null } }
let pendingUpdate: Update | null = null
const pricing: PricingCatalogStatus = { version: '2026-09-06.1', currency: 'USD', verifiedAt: '2026-09-06', pricedObservations: 97, totalObservations: 106, rates: [
  { vendor: 'OpenAI', model: 'gpt-5.6-sol', aliases: ['gpt-5.6'], currency: 'USD', inputUsdPerMillion: '4', cachedReadUsdPerMillion: '0.4', cachedWriteUsdPerMillion: '5', outputUsdPerMillion: '20', effectiveFrom: '2026-01-01', effectiveTo: null, sourceUrl: 'https://developers.openai.com/api/docs/models/gpt-5.6-sol', verifiedAt: '2026-09-06' },
  { vendor: 'MiniMax', model: 'MiniMax-M2.7', aliases: [], currency: 'USD', inputUsdPerMillion: '0.3', cachedReadUsdPerMillion: '0.06', cachedWriteUsdPerMillion: '0.375', outputUsdPerMillion: '1.2', effectiveFrom: '2026-01-01', effectiveTo: null, sourceUrl: 'https://platform.minimax.io/docs/guides/pricing-paygo', verifiedAt: '2026-09-06' },
] }
const call = async <T>(command: string, args?: Record<string, unknown>, fallback?: () => T): Promise<T> => { if (isTauri()) return invoke<T>(command, args); await new Promise(resolve => setTimeout(resolve, 60)); if (!fallback) throw new Error(`Mock not implemented: ${command}`); return fallback() }
const filtered = (filters: AnalyticsFilters) => matrix.filter(row => (!filters.sourceId || row.sourceId === filters.sourceId) && (!filters.model || row.model === filters.model) && (!filters.reasoningEffort || row.reasoningEffort === filters.reasoningEffort))
const mockHasUpdate = () => typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('update') === 'available'

export const meterApi = {
  isTauri,
  discoverSources: () => call<SourceInfo[]>('discover_sources', undefined, () => structuredClone(sources)),
  startImport: (force = false) => call<ImportStatus>('start_import', { force }, () => (mockStatus = { ...mockStatus, running: true, paused: false, message: force ? '正在重建索引' : '正在同步三个数据源' })),
  pauseImport: () => call<ImportStatus>('pause_import', undefined, () => (mockStatus = { ...mockStatus, running: false, paused: true, message: '导入已暂停' })),
  getImportStatus: () => call<ImportStatus>('get_import_status', undefined, () => ({ ...mockStatus })),
  queryMetricSummary: (filters: AnalyticsFilters) => call<MetricSummary>('query_metric_summary', { filters }, () => ({ ...summarize(filtered(filters)), period: filters.period })),
  queryMetricSeries: (filters: AnalyticsFilters) => call<MetricSeriesPoint[]>('query_metric_series', { filters }, () => structuredClone(series)),
  queryModelEffortStats: (filters: AnalyticsFilters) => call<ModelEffortStat[]>('query_model_effort_stats', { filters }, () => structuredClone(filtered(filters))),
  getPricingCatalogStatus: () => call<PricingCatalogStatus>('get_pricing_catalog_status', undefined, () => structuredClone(pricing)),
  repriceUsage: () => call<PricingCatalogStatus>('reprice_usage', undefined, () => structuredClone(pricing)),
  updateSource: (id: number, enabled: boolean) => call<SourceInfo>('update_source', { id, enabled }, () => { const source = sources.find(item => item.id === id)!; source.enabled = enabled; return { ...source } }),
  getAppSettings: () => call<AppSettings>('get_app_settings', undefined, () => structuredClone(mockSettings)),
  updateAppSettings: (settings: AppSettings) => call<AppSettings>('update_app_settings', { settings }, () => (mockSettings = structuredClone(settings))),
  getCurrentVersion: async () => isTauri() ? getVersion() : '0.3.0',
  checkForUpdate: async (): Promise<AppUpdateState> => { const currentVersion = await meterApi.getCurrentVersion(); if (!isTauri()) return mockHasUpdate() ? { phase: 'available', currentVersion, version: '0.3.1', notes: '稳定性改进。', downloadedBytes: 0 } : { phase: 'current', currentVersion, downloadedBytes: 0 }; if (pendingUpdate) { await pendingUpdate.close(); pendingUpdate = null }; const update = await check({ timeout: 20_000 }); if (!update) return { phase: 'current', currentVersion, downloadedBytes: 0 }; pendingUpdate = update; return { phase: 'available', currentVersion, version: update.version, notes: update.body, downloadedBytes: 0 } },
  downloadAndInstallUpdate: async (onState: (state: AppUpdateState) => void) => { const currentVersion = await meterApi.getCurrentVersion(); if (!isTauri()) { onState({ phase: 'ready', currentVersion, version: '0.3.1', downloadedBytes: 1, totalBytes: 1 }); return }; if (!pendingUpdate) throw new Error('没有可安装的更新，请先检查更新'); const update = pendingUpdate; let downloadedBytes = 0; let totalBytes: number | undefined; await update.download(event => { if (event.event === 'Started') totalBytes = event.data.contentLength; if (event.event === 'Progress') downloadedBytes += event.data.chunkLength; onState({ phase: 'downloading', currentVersion, version: update.version, notes: update.body, downloadedBytes, totalBytes }) }, { timeout: 120_000 }); await meterApi.pauseImport(); await update.install(); onState({ phase: 'ready', currentVersion, version: update.version, downloadedBytes, totalBytes }); await relaunch() },
  isAutostartEnabled: async () => isTauri() ? isEnabled() : mockAutostart,
  setAutostartEnabled: async (enabledValue: boolean) => { if (isTauri()) enabledValue ? await enable() : await disable(); else mockAutostart = enabledValue; return enabledValue },
  on: async <T>(event: MeterEvent, handler: (payload: T) => void): Promise<UnlistenFn> => isTauri() ? listen<T>(event, ({ payload }) => handler(payload)) : () => undefined,
}
