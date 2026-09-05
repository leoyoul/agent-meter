export type AgentKind = 'all' | 'root' | 'subagent'

export interface MetricFilters {
  startDate?: string
  endDate?: string
  sourceId?: number
  model?: string
  project?: string
  agentKind?: AgentKind
}

export interface SourceInfo {
  id: number
  name: string
  rootPath: string
  enabled: boolean
  available: boolean
  fileCount: number
  totalBytes: number
  lastScanAt: string | null
  error: string | null
}

export interface ImportStatus {
  running: boolean
  paused: boolean
  filesDone: number
  filesTotal: number
  bytesDone: number
  bytesTotal: number
  currentFile: string | null
  startedAt: string | null
  message: string
}

export interface TokenBreakdown {
  input: number
  cachedInput: number
  output: number
  reasoning: number
  total: number
}

export interface Overview {
  tokens: TokenBreakdown
  turnCount: number
  runningCount: number
  subagentCount: number
  medianTtftMs: number | null
  p95TtftMs: number | null
  medianDurationMs: number | null
  p95DurationMs: number | null
  medianEffectiveTps: number | null
  recentMedianTtftMs: number | null
  recentMedianEffectiveTps: number | null
  lastUpdatedAt: string | null
}

export interface TimeseriesPoint {
  bucket: string
  inputTokens: number
  cachedInputTokens: number
  outputTokens: number
  reasoningTokens: number
  totalTokens: number
  medianTtftMs: number | null
  medianEffectiveTps: number | null
  turnCount: number
}

export interface ModelStat {
  model: string
  reasoningEffort: string | null
  turnCount: number
  tokens: TokenBreakdown
  medianTtftMs: number | null
  p95TtftMs: number | null
  medianDurationMs: number | null
  p95DurationMs: number | null
  medianEffectiveTps: number | null
}

export interface TaskRow {
  turnId: string
  sessionId: string
  parentThreadId: string | null
  sourceName: string
  project: string
  cwd: string
  model: string
  reasoningEffort: string | null
  agentKind: 'root' | 'subagent'
  agentPath: string | null
  startedAt: string
  completedAt: string | null
  durationMs: number | null
  ttftMs: number | null
  effectiveTps: number | null
  tokens: TokenBreakdown
  status: 'running' | 'completed' | 'incomplete'
}
