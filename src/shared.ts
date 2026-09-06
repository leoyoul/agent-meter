export type MetricPeriod = 'realtime' | 'today' | 'week' | 'month' | 'year'

export interface AppSettings {
  menuMetrics: { todayTokens: boolean; ttft: boolean; effectiveTps: boolean; estimatedCost: boolean }
  menuPeriod: MetricPeriod
  updates: { automaticCheck: boolean; lastCheckedAt?: string | null }
}

export type UpdatePhase = 'idle' | 'checking' | 'available' | 'current' | 'downloading' | 'ready' | 'error'
export interface AppUpdateState { phase: UpdatePhase; currentVersion: string; version?: string; notes?: string; downloadedBytes: number; totalBytes?: number; error?: string }
export interface AnalyticsFilters { period: MetricPeriod; sourceId?: number; model?: string; reasoningEffort?: string }
export interface SourceInfo { id: number; name: string; rootPath: string; enabled: boolean; available: boolean; fileCount: number; totalBytes: number; lastScanAt: string | null; error: string | null }
export interface ImportStatus { running: boolean; paused: boolean; filesDone: number; filesTotal: number; bytesDone: number; bytesTotal: number; currentFile: string | null; startedAt: string | null; message: string }
export interface UsageTokens { uncachedInput: number; cachedRead: number; cachedWrite: number; output: number; reasoning: number; total: number }
export interface PricingCoverage { pricedObservations: number; totalObservations: number; pricedTokens: number; totalTokens: number; ratio: number; complete: boolean }
export interface MetricSummary { period: MetricPeriod; observationCount: number; averageTtftMs: number | null; averageEffectiveTps: number | null; tokens: UsageTokens; estimatedCostNanoUsd: number; pricing: PricingCoverage; lastUpdatedAt: string | null }
export interface MetricSeriesPoint { bucket: string; label: string; observationCount: number; averageTtftMs: number | null; averageEffectiveTps: number | null; totalTokens: number; estimatedCostNanoUsd: number }
export interface ModelEffortStat { sourceId: number; sourceName: string; provider: string; model: string; reasoningEffort: string; observationCount: number; averageTtftMs: number | null; averageEffectiveTps: number | null; tokens: UsageTokens; estimatedCostNanoUsd: number; pricing: PricingCoverage }
export interface PricingRate { vendor: string; model: string; aliases: string[]; currency: string; inputUsdPerMillion: string; cachedReadUsdPerMillion: string | null; cachedWriteUsdPerMillion: string | null; outputUsdPerMillion: string; effectiveFrom: string; effectiveTo: string | null; sourceUrl: string; verifiedAt: string }
export interface PricingCatalogStatus { version: string; currency: string; verifiedAt: string; rates: PricingRate[]; pricedObservations: number; totalObservations: number }
