import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
import type {
  ImportStatus,
  MetricFilters,
  ModelStat,
  Overview,
  SourceInfo,
  TaskRow,
  TimeseriesPoint,
} from './shared'

export type MeterEvent = 'import-progress' | 'metrics-updated' | 'source-error'

const isTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const now = new Date()
const iso = (offsetMinutes = 0) => new Date(now.getTime() + offsetMinutes * 60_000).toISOString()

const sources: SourceInfo[] = [
  { id: 1, name: 'Codex', rootPath: '~/.codex', enabled: true, available: true, fileCount: 546, totalBytes: 2_577_980_416, lastScanAt: iso(-2), error: null },
  { id: 2, name: 'Yodex', rootPath: '~/.yodex', enabled: true, available: true, fileCount: 2, totalBytes: 4_718_592, lastScanAt: iso(-8), error: null },
]

const modelStats: ModelStat[] = [
  { model: 'codex-pro', reasoningEffort: 'high', turnCount: 84, tokens: { input: 1_842_300, cachedInput: 1_224_900, output: 184_600, reasoning: 52_300, total: 2_079_200 }, medianTtftMs: 870, p95TtftMs: 1640, medianDurationMs: 38_400, p95DurationMs: 126_000, medianEffectiveTps: 12.7 },
  { model: 'codex-balanced', reasoningEffort: 'medium', turnCount: 61, tokens: { input: 906_800, cachedInput: 601_200, output: 97_400, reasoning: 31_900, total: 1_036_100 }, medianTtftMs: 690, p95TtftMs: 1280, medianDurationMs: 26_900, p95DurationMs: 89_000, medianEffectiveTps: 15.4 },
  { model: 'codex-fast', reasoningEffort: 'medium', turnCount: 112, tokens: { input: 1_214_000, cachedInput: 823_500, output: 143_700, reasoning: 28_600, total: 1_386_300 }, medianTtftMs: 510, p95TtftMs: 940, medianDurationMs: 19_300, p95DurationMs: 64_000, medianEffectiveTps: 18.9 },
]

const tasks: TaskRow[] = [
  { turnId: 'turn-01', sessionId: 'demo-root-01', parentThreadId: null, sourceName: 'Codex', project: 'demo-dashboard', cwd: '/Users/demo/Projects/demo-dashboard', model: 'codex-pro', reasoningEffort: 'high', agentKind: 'root', agentPath: '/root', startedAt: iso(-13), completedAt: iso(-2), durationMs: 658_000, ttftMs: 920, effectiveTps: 11.8, tokens: { input: 248_400, cachedInput: 181_000, output: 22_600, reasoning: 6_800, total: 277_800 }, status: 'completed' },
  { turnId: 'turn-02', sessionId: 'demo-sub-01', parentThreadId: 'demo-root-01', sourceName: 'Codex', project: 'demo-dashboard', cwd: '/Users/demo/Projects/demo-dashboard', model: 'codex-fast', reasoningEffort: 'medium', agentKind: 'subagent', agentPath: '/root/frontend_agent', startedAt: iso(-11), completedAt: iso(-4), durationMs: 421_000, ttftMs: 480, effectiveTps: 19.6, tokens: { input: 142_800, cachedInput: 96_000, output: 18_700, reasoning: 3_600, total: 165_100 }, status: 'completed' },
  { turnId: 'turn-03', sessionId: 'demo-sub-02', parentThreadId: 'demo-root-01', sourceName: 'Codex', project: 'demo-dashboard', cwd: '/Users/demo/Projects/demo-dashboard', model: 'codex-balanced', reasoningEffort: 'high', agentKind: 'subagent', agentPath: '/root/parser_agent', startedAt: iso(-9), completedAt: null, durationMs: null, ttftMs: 720, effectiveTps: null, tokens: { input: 91_400, cachedInput: 66_200, output: 7_900, reasoning: 2_100, total: 101_400 }, status: 'running' },
  { turnId: 'turn-04', sessionId: 'demo-root-02', parentThreadId: null, sourceName: 'Yodex', project: 'sample-service', cwd: '/Users/demo/Projects/sample-service', model: 'codex-fast', reasoningEffort: 'medium', agentKind: 'root', agentPath: '/root', startedAt: iso(-240), completedAt: iso(-218), durationMs: 1_320_000, ttftMs: 530, effectiveTps: 17.2, tokens: { input: 184_300, cachedInput: 120_400, output: 19_300, reasoning: 4_100, total: 207_700 }, status: 'completed' },
]

const overview: Overview = {
  tokens: { input: 1_186_420, cachedInput: 784_210, output: 126_840, reasoning: 31_960, total: 1_345_220 },
  turnCount: 47, runningCount: 1, subagentCount: 18,
  medianTtftMs: 720, p95TtftMs: 1480, medianDurationMs: 31_600, p95DurationMs: 104_000,
  medianEffectiveTps: 15.8, recentMedianTtftMs: 680, recentMedianEffectiveTps: 16.4, lastUpdatedAt: iso(-1),
}

const timeseries: TimeseriesPoint[] = Array.from({ length: 14 }, (_, i) => {
  const date = new Date(now); date.setDate(now.getDate() - 13 + i)
  const wave = [62, 78, 55, 91, 104, 82, 48, 118, 94, 126, 88, 139, 107, 131][i] * 10_000
  return { bucket: date.toISOString().slice(0, 10), inputTokens: Math.round(wave * .82), cachedInputTokens: Math.round(wave * .56), outputTokens: Math.round(wave * .13), reasoningTokens: Math.round(wave * .05), totalTokens: wave, medianTtftMs: 520 + i * 17, medianEffectiveTps: 13 + (i % 5) * 1.3, turnCount: 12 + i * 2 }
})

let mockStatus: ImportStatus = { running: false, paused: false, filesDone: 546, filesTotal: 546, bytesDone: 2_577_980_416, bytesTotal: 2_577_980_416, currentFile: null, startedAt: iso(-48), message: '索引已是最新' }
let mockAutostart = false

const call = async <T>(command: string, args?: Record<string, unknown>, fallback?: () => T): Promise<T> => {
  if (isTauri()) return invoke<T>(command, args)
  await new Promise(resolve => setTimeout(resolve, 120))
  if (!fallback) throw new Error(`Mock not implemented: ${command}`)
  return fallback()
}

const filterTasks = (filters: MetricFilters) => tasks.filter(task =>
  (!filters.sourceId || sources.find(source => source.id === filters.sourceId)?.name === task.sourceName) &&
  (!filters.model || task.model === filters.model) &&
  (!filters.project || task.project === filters.project) &&
  (!filters.agentKind || filters.agentKind === 'all' || task.agentKind === filters.agentKind),
)

export const meterApi = {
  isTauri,
  discoverSources: () => call<SourceInfo[]>('discover_sources', undefined, () => sources.map(source => ({ ...source }))),
  startImport: (force = false) => call<ImportStatus>('start_import', { force }, () => {
    mockStatus = { ...mockStatus, running: true, paused: false, startedAt: new Date().toISOString(), message: force ? '正在重建索引' : '正在检查新记录' }
    return { ...mockStatus }
  }),
  pauseImport: () => call<ImportStatus>('pause_import', undefined, () => {
    mockStatus = { ...mockStatus, running: false, paused: true, message: '导入已暂停' }
    return { ...mockStatus }
  }),
  getImportStatus: () => call<ImportStatus>('get_import_status', undefined, () => ({ ...mockStatus })),
  queryOverview: (filters: MetricFilters) => call<Overview>('query_overview', { filters }, () => ({ ...overview })),
  queryTimeseries: (filters: MetricFilters) => call<TimeseriesPoint[]>('query_timeseries', { filters }, () => timeseries.map(point => ({ ...point }))),
  queryModelStats: (filters: MetricFilters) => call<ModelStat[]>('query_model_stats', { filters }, () => modelStats.filter(row => !filters.model || row.model === filters.model).map(row => ({ ...row, tokens: { ...row.tokens } }))),
  queryTasks: (filters: MetricFilters) => call<TaskRow[]>('query_tasks', { filters }, () => filterTasks(filters).map(task => ({ ...task, tokens: { ...task.tokens } }))),
  updateSource: (id: number, enabled: boolean) => call<SourceInfo>('update_source', { id, enabled }, () => {
    const source = sources.find(item => item.id === id)!
    source.enabled = enabled
    return { ...source }
  }),
  isAutostartEnabled: async () => isTauri() ? isEnabled() : mockAutostart,
  setAutostartEnabled: async (enabled: boolean) => {
    if (isTauri()) enabled ? await enable() : await disable()
    else mockAutostart = enabled
    return enabled
  },
  on: async <T>(event: MeterEvent, handler: (payload: T) => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen<T>(event, ({ payload }) => handler(payload))
    return () => undefined
  },
}
