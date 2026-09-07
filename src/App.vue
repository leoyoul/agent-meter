<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Activity, BarChart3, Check, ChevronDown, CircleAlert, CloudDownload, Coins, Database, Gauge, Info, LoaderCircle, Pause, Pencil, Play, Plus, RefreshCw, Search, Settings, Timer, Trash2, X, Zap } from 'lucide-vue-next'
import { meterApi } from './api'
import type { AnalyticsFilters, AppSettings, AppUpdateState, DataIntegrityStatus, ImportStatus, MetricPeriod, MetricSeriesPoint, MetricSummary, ModelEffortStat, PricingCatalogStatus, PricingModel, PricingRate, PricingRateInput, SourceInfo } from './shared'

const periods: Array<[MetricPeriod, string]> = [['realtime', '实时'], ['today', '今日'], ['week', '本周'], ['month', '本月'], ['year', '本年']]
const isSettingsView = new URLSearchParams(window.location.search).get('view') === 'settings'
const filters = ref<AnalyticsFilters>({ period: 'realtime' })
const summary = ref<MetricSummary | null>(null)
const series = ref<MetricSeriesPoint[]>([])
const stats = ref<ModelEffortStat[]>([])
const sources = ref<SourceInfo[]>([])
const pricing = ref<PricingCatalogStatus | null>(null)
const pricingModels = ref<PricingModel[]>([])
const integrity = ref<DataIntegrityStatus | null>(null)
const viewMode = ref<'analysis' | 'pricing'>('analysis')
const priceSearch = ref('')
const pricingEditor = ref(false)
const editingRateId = ref<number | null>(null)
const priceForm = ref<PricingRateInput>({ vendor: '', model: '', aliases: [], inputUsdPerMillion: '', cachedReadUsdPerMillion: null, cachedWriteUsdPerMillion: null, outputUsdPerMillion: '', effectiveFrom: new Date().toISOString().slice(0, 10), effectiveTo: null, sourceUrl: '' })
const importStatus = ref<ImportStatus | null>(null)
const loading = ref(true)
const error = ref('')
const lastAction = ref('')
const autostartEnabled = ref(false)
const appSettings = ref<AppSettings>({ menuMetrics: { todayTokens: true, ttft: false, effectiveTps: false, estimatedCost: false }, showAppIcon: false, menuPeriod: 'realtime', activeSourceKind: null, updates: { automaticCheck: true, lastCheckedAt: null } })
const updateState = ref<AppUpdateState>({ phase: 'idle', currentVersion: '0.4.2', downloadedBytes: 0 })
const unlisteners: Array<() => void> = []
let updateDelay: ReturnType<typeof setTimeout> | undefined
let updateInterval: ReturnType<typeof setInterval> | undefined

const modelNames = computed(() => [...new Set(stats.value.map(row => row.model))].sort())
const efforts = computed(() => [...new Set(stats.value.map(row => row.reasoningEffort))].sort())
const activeSource = computed(() => sources.value.find(item => item.sourceKind === appSettings.value.activeSourceKind) ?? null)
const sourceName = computed(() => activeSource.value?.name ?? '全部来源')
const sourceUnavailable = computed(() => activeSource.value && (!activeSource.value.available || !activeSource.value.enabled || activeSource.value.dataCapability === 'noUsageLog'))
const sourceUnavailableReason = computed(() => activeSource.value?.limitation || (!activeSource.value?.available ? '本机未发现该数据源' : !activeSource.value?.enabled ? '该数据源已在设置中停用' : '该来源暂时没有可统计指标'))
const maxSeries = computed(() => Math.max(...series.value.map(point => point.totalTokens), 1))
const chartPoints = computed(() => series.value.map((point, index) => `${series.value.length <= 1 ? 50 : index / (series.value.length - 1) * 100},${92 - point.totalTokens / maxSeries.value * 80}`).join(' '))
const chartArea = computed(() => `0,100 ${chartPoints.value} 100,100`)
const updateProgress = computed(() => updateState.value.totalBytes ? Math.min(100, updateState.value.downloadedBytes / updateState.value.totalBytes * 100) : 0)
const periodLabel = computed(() => periods.find(([key]) => key === filters.value.period)?.[1] ?? '实时')
const pricedPercent = computed(() => Math.round((summary.value?.pricing.ratio ?? 1) * 100))
const filteredPricingModels = computed(() => { const query = priceSearch.value.trim().toLowerCase(); return pricingModels.value.filter(item => !query || `${item.vendor} ${item.model}`.toLowerCase().includes(query)) })
const formatCompact = (value: number) => new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
const formatDuration = (value: number | null | undefined) => value == null ? '—' : value < 1000 ? `${Math.round(value)} ms` : `${(value / 1000).toFixed(1)} s`
const formatTps = (value: number | null | undefined) => value == null ? '—' : value.toFixed(1)
const formatCost = (nano: number, complete = true) => `${new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: nano >= 10_000_000_000 ? 0 : 2, maximumFractionDigits: nano >= 1_000_000_000 ? 2 : 4 }).format(nano / 1_000_000_000)}${complete ? '' : '+'}`
const formatTime = (value: string | null | undefined) => value ? new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(new Date(value)) : '—'
const formatBytes = (value: number) => value >= 1_073_741_824 ? `${(value / 1_073_741_824).toFixed(1)} GB` : `${Math.max(0, value / 1_048_576).toFixed(value < 10_485_760 ? 1 : 0)} MB`
const effortName = (value: string) => ({ default: '默认', unknown: '未知' }[value] ?? value)
const cloneSettings = (value: AppSettings): AppSettings => ({ menuMetrics: { ...value.menuMetrics }, showAppIcon: value.showAppIcon, menuPeriod: value.menuPeriod, activeSourceKind: value.activeSourceKind ?? null, updates: { ...value.updates } })

async function loadAll(silent = false) {
  if (isSettingsView) return
  if (!silent) loading.value = true
  error.value = ''
  try {
    const next = { ...filters.value }
    const [nextSummary, nextSeries, nextStats, nextStatus, nextIntegrity] = await Promise.all([meterApi.queryMetricSummary(next), meterApi.queryMetricSeries(next), meterApi.queryModelEffortStats(next), meterApi.getImportStatus(), meterApi.getDataIntegrityStatus()])
    summary.value = nextSummary; series.value = nextSeries; stats.value = nextStats; importStatus.value = nextStatus; integrity.value = nextIntegrity
  } catch (reason) { error.value = reason instanceof Error ? reason.message : String(reason) }
  finally { loading.value = false }
}
async function saveSettings(next: AppSettings, message?: string) { try { appSettings.value = await meterApi.updateAppSettings(next); if (message) lastAction.value = message } catch (reason) { error.value = String(reason) } }
async function selectPeriod(period: MetricPeriod) { filters.value.period = period; const next = cloneSettings(appSettings.value); next.menuPeriod = period; await saveSettings(next) }
async function selectSource(source: SourceInfo | null) { const next = cloneSettings(appSettings.value); next.activeSourceKind = source?.sourceKind ?? null; filters.value.sourceId = source?.id; filters.value.model = undefined; filters.value.reasoningEffort = undefined; await saveSettings(next) }
function navigateSource(event: KeyboardEvent, index: number) { const tabs: Array<SourceInfo | null> = [null, ...sources.value]; const nextIndex = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : -1; if (nextIndex < 0) return; event.preventDefault(); void selectSource(tabs[nextIndex]); document.querySelectorAll<HTMLButtonElement>('.source-tab')[nextIndex]?.focus() }
async function toggleAppIcon() { const next = cloneSettings(appSettings.value); next.showAppIcon = !next.showAppIcon; await saveSettings(next, `应用图标已${next.showAppIcon ? '显示' : '隐藏'}`) }
async function toggleMenuMetric(metric: keyof AppSettings['menuMetrics']) { const next = cloneSettings(appSettings.value); next.menuMetrics[metric] = !next.menuMetrics[metric]; const names = { todayTokens: '量', ttft: '首', effectiveTps: '速', estimatedCost: '费' }; await saveSettings(next, `${names[metric]}已${next.menuMetrics[metric] ? '显示' : '隐藏'}`) }
async function toggleSource(source: SourceInfo) { try { const updated = await meterApi.updateSource(source.id, !source.enabled); sources.value = sources.value.map(item => item.id === updated.id ? updated : item); await loadAll(true) } catch (reason) { error.value = String(reason) } }
async function runImport(force = false) { try { importStatus.value = await meterApi.startImport(force); lastAction.value = force ? '已开始重建索引' : '已开始同步' } catch (reason) { error.value = String(reason) } }
async function pauseImport() { try { importStatus.value = await meterApi.pauseImport() } catch (reason) { error.value = String(reason) } }
async function reprice() { try { pricing.value = await meterApi.repriceUsage(); await loadAll(true); lastAction.value = '已按当前价目重新计价' } catch (reason) { error.value = String(reason) } }
async function loadPricing() { pricingModels.value = await meterApi.listPricingModels(); pricing.value = await meterApi.getPricingCatalogStatus() }
function openPriceEditor(rate?: PricingRate, model?: PricingModel) { editingRateId.value = rate?.id ?? null; priceForm.value = rate ? { vendor: rate.vendor, model: rate.model, aliases: [...rate.aliases], inputUsdPerMillion: rate.inputUsdPerMillion, cachedReadUsdPerMillion: rate.cachedReadUsdPerMillion, cachedWriteUsdPerMillion: rate.cachedWriteUsdPerMillion, outputUsdPerMillion: rate.outputUsdPerMillion, effectiveFrom: rate.effectiveFrom, effectiveTo: rate.effectiveTo, sourceUrl: rate.sourceUrl } : { vendor: model?.vendor === 'unknown' ? '' : model?.vendor ?? '', model: model?.model ?? '', aliases: [], inputUsdPerMillion: '', cachedReadUsdPerMillion: null, cachedWriteUsdPerMillion: null, outputUsdPerMillion: '', effectiveFrom: new Date().toISOString().slice(0, 10), effectiveTo: null, sourceUrl: '' }; pricingEditor.value = true }
async function savePrice() { try { if (editingRateId.value == null) await meterApi.createPricingRate(priceForm.value); else await meterApi.updatePricingRate(editingRateId.value, priceForm.value); pricingEditor.value = false; await loadPricing(); await loadAll(true); lastAction.value = '价格已保存并重算历史费用' } catch (reason) { error.value = String(reason) } }
async function deletePrice(rate: PricingRate) { if (!window.confirm(`删除 ${rate.model} 自 ${rate.effectiveFrom} 生效的价格版本？`)) return; try { await meterApi.deletePricingRate(rate.id); await loadPricing(); await loadAll(true); lastAction.value = '价格版本已删除，相关调用标记为未计价' } catch (reason) { error.value = String(reason) } }
async function toggleAutostart() { try { autostartEnabled.value = await meterApi.setAutostartEnabled(!autostartEnabled.value) } catch (reason) { error.value = String(reason) } }
async function toggleAutomaticUpdates() { const next = cloneSettings(appSettings.value); next.updates.automaticCheck = !next.updates.automaticCheck; await saveSettings(next) }
async function checkForUpdates(silent = false) { if (['checking', 'downloading'].includes(updateState.value.phase)) return; updateState.value = { ...updateState.value, phase: 'checking', error: undefined }; try { updateState.value = await meterApi.checkForUpdate(); const next = cloneSettings(appSettings.value); next.updates.lastCheckedAt = new Date().toISOString(); appSettings.value = await meterApi.updateAppSettings(next); if (!silent && updateState.value.phase === 'current') lastAction.value = 'Agent Meter 已是最新版' } catch (reason) { updateState.value = { ...updateState.value, phase: 'error', error: reason instanceof Error ? reason.message : String(reason) } } }
async function downloadAndRestart() { try { await meterApi.downloadAndInstallUpdate(state => { updateState.value = state }) } catch (reason) { updateState.value = { ...updateState.value, phase: 'error', error: String(reason) } } }

watch(filters, () => loadAll(), { deep: true })
onMounted(async () => {
  document.body.dataset.view = isSettingsView ? 'settings' : 'dashboard'
  const [nextSources, nextAutostart, nextSettings, nextPricing, nextPricingModels, currentVersion, nextStatus] = await Promise.all([meterApi.discoverSources(), meterApi.isAutostartEnabled(), meterApi.getAppSettings(), meterApi.getPricingCatalogStatus(), meterApi.listPricingModels(), meterApi.getCurrentVersion(), meterApi.getImportStatus()])
  sources.value = nextSources; autostartEnabled.value = nextAutostart; appSettings.value = nextSettings; pricing.value = nextPricing; pricingModels.value = nextPricingModels; importStatus.value = nextStatus; filters.value.period = nextSettings.menuPeriod; filters.value.sourceId = nextSources.find(item => item.sourceKind === nextSettings.activeSourceKind)?.id; updateState.value.currentVersion = currentVersion
  await loadAll()
  unlisteners.push(await meterApi.on<ImportStatus>('import-progress', payload => { importStatus.value = payload }))
  unlisteners.push(await meterApi.on('metrics-updated', () => loadAll(true)))
  unlisteners.push(await meterApi.on<string>('source-error', payload => { error.value = payload }))
  unlisteners.push(await meterApi.on<AppSettings>('settings-updated', payload => { appSettings.value = payload; filters.value.period = payload.menuPeriod; filters.value.sourceId = sources.value.find(item => item.sourceKind === payload.activeSourceKind)?.id }))
  unlisteners.push(await meterApi.on('sources-updated', async () => { sources.value = await meterApi.discoverSources() }))
  if (!isSettingsView && meterApi.isTauri()) { updateDelay = setTimeout(() => { if (appSettings.value.updates.automaticCheck) void checkForUpdates(true) }, 15_000); updateInterval = setInterval(() => { if (appSettings.value.updates.automaticCheck) void checkForUpdates(true) }, 86_400_000) }
})
onBeforeUnmount(() => { unlisteners.splice(0).forEach(fn => fn()); if (updateDelay) clearTimeout(updateDelay); if (updateInterval) clearInterval(updateInterval) })
</script>

<template>
  <main v-if="!isSettingsView" class="app-shell">
    <header class="topbar">
      <div class="brand"><div class="brand-mark"><Activity :size="20" /></div><div><strong>Agent Meter</strong><span>本机 Agent 四指标</span></div></div>
      <div class="header-actions"><span class="freshness"><i :class="{ live: importStatus?.running }"></i>{{ importStatus?.running ? '正在同步' : `更新于 ${formatTime(summary?.lastUpdatedAt)}` }}</span><button class="icon-button" title="刷新数据" aria-label="刷新数据" @click="runImport(false)"><RefreshCw :size="18" :class="{ spin: importStatus?.running }" /></button><button class="icon-button" title="设置" aria-label="打开设置窗口" @click="meterApi.showSettingsWindow()"><Settings :size="18" /></button></div>
    </header>

    <nav class="view-tabs segmented" aria-label="主视图"><button :class="{ active: viewMode === 'analysis' }" @click="viewMode = 'analysis'">分析</button><button :class="{ active: viewMode === 'pricing' }" @click="viewMode = 'pricing'">价格</button></nav>

    <template v-if="viewMode === 'analysis'">

    <nav class="source-tabs" role="tablist" aria-label="Agent 来源">
      <button class="source-tab" role="tab" :aria-selected="!appSettings.activeSourceKind" :class="{ active: !appSettings.activeSourceKind }" @click="selectSource(null)" @keydown="navigateSource($event, 0)">全部</button>
      <button v-for="(source, index) in sources" :key="source.id" class="source-tab" role="tab" :aria-selected="appSettings.activeSourceKind === source.sourceKind" :class="{ active: appSettings.activeSourceKind === source.sourceKind, unavailable: !source.available || !source.enabled || source.dataCapability === 'noUsageLog' }" :title="source.limitation || undefined" @click="selectSource(source)" @keydown="navigateSource($event, index + 1)"><i></i>{{ source.name }}</button>
    </nav>

    <section class="control-band" aria-label="统计筛选">
      <div class="period-control segmented"><button v-for="[key, label] in periods" :key="key" :class="{ active: filters.period === key }" @click="selectPeriod(key)">{{ label }}</button></div>
      <label><span>模型</span><div class="select-wrap"><select v-model="filters.model"><option value="">全部模型</option><option v-for="name in modelNames" :key="name">{{ name }}</option></select><ChevronDown :size="15" /></div></label>
      <label><span>推理强度</span><div class="select-wrap"><select v-model="filters.reasoningEffort"><option value="">全部强度</option><option v-for="effort in efforts" :key="effort" :value="effort">{{ effortName(effort) }}</option></select><ChevronDown :size="15" /></div></label>
    </section>

    <div v-if="error" class="notice error"><CircleAlert :size="18" />{{ error }}<button aria-label="关闭错误" @click="error = ''"><X :size="16" /></button></div>
    <div v-if="lastAction" class="notice success"><Check :size="18" />{{ lastAction }}<button aria-label="关闭提示" @click="lastAction = ''"><X :size="16" /></button></div>

    <section v-if="sourceUnavailable" class="source-unavailable"><Database :size="28"/><div><span>{{ activeSource?.name }}</span><h1>暂无可统计数据</h1><p>{{ sourceUnavailableReason }}</p></div><button class="secondary-button" @click="meterApi.showSettingsWindow()"><Settings :size="16"/>打开设置</button></section>

    <section v-if="!sourceUnavailable" class="kpi-grid" aria-label="四大指标">
      <article class="kpi speed"><div class="metric-code">速</div><span>{{ periodLabel }}平均有效 TPS</span><strong>{{ formatTps(summary?.averageEffectiveTps) }}</strong><small>{{ summary?.performanceSampleCount ?? 0 }} 个有效性能样本</small></article>
      <article class="kpi first"><div class="metric-code">首</div><span>{{ periodLabel }}平均首响</span><strong>{{ formatDuration(summary?.averageTtftMs) }}</strong><small>{{ summary?.performanceSampleCount ?? 0 }} 个有效性能样本</small></article>
      <article class="kpi volume"><div class="metric-code">量</div><span>{{ periodLabel }} Token 累计</span><strong>{{ formatCompact(summary?.tokens.total ?? 0) }}</strong><small>非缓存输入 + 缓存读写 + 输出</small></article>
      <article class="kpi cost"><div class="metric-code">费</div><span>{{ periodLabel }} API 等价费用</span><strong>{{ formatCost(summary?.estimatedCostNanoUsd ?? 0, summary?.pricing.complete) }}</strong><small>USD · 计价覆盖 {{ pricedPercent }}%</small></article>
    </section>

    <section v-if="!sourceUnavailable" class="content-grid">
      <article class="panel chart-panel"><div class="panel-heading"><div><span class="eyebrow">趋势</span><h2>{{ periodLabel }}用量走势</h2></div><span class="source-pill">{{ sourceName }}</span></div><div v-if="loading" class="loading"><LoaderCircle class="spin" :size="22" />正在读取本机指标</div><div v-else-if="!series.length" class="empty">当前筛选没有可用数据</div><div v-else class="chart-wrap"><div class="axis"><span>{{ formatCompact(maxSeries) }}</span><span>{{ formatCompact(maxSeries / 2) }}</span><span>0</span></div><svg viewBox="0 0 100 100" preserveAspectRatio="none" role="img" aria-label="Token 用量趋势"><defs><linearGradient id="area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".28"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs><line v-for="y in [12,52,92]" :key="y" x1="0" :y1="y" x2="100" :y2="y" class="gridline"/><polygon :points="chartArea" fill="url(#area)"/><polyline :points="chartPoints" class="chart-line"/></svg><div class="chart-labels"><span v-for="(point, index) in series" v-show="index % Math.max(1, Math.ceil(series.length / 5)) === 0 || index === series.length - 1" :key="point.bucket">{{ point.label }}</span></div></div></article>
      <article class="panel breakdown-panel"><div class="panel-heading"><div><span class="eyebrow">TOKEN 构成</span><h2>互不重叠的计量桶</h2></div><BarChart3 :size="20" /></div><div class="breakdown-total"><strong>{{ formatCompact(summary?.tokens.total ?? 0) }}</strong><span>总 Token</span></div><div class="breakdown-list"><div><span><i class="dot blue"></i>非缓存输入</span><strong>{{ formatCompact(summary?.tokens.uncachedInput ?? 0) }}</strong></div><div><span><i class="dot cyan"></i>缓存读取</span><strong>{{ formatCompact(summary?.tokens.cachedRead ?? 0) }}</strong></div><div><span><i class="dot amber"></i>缓存写入</span><strong>{{ formatCompact(summary?.tokens.cachedWrite ?? 0) }}</strong></div><div><span><i class="dot green"></i>输出</span><strong>{{ formatCompact(summary?.tokens.output ?? 0) }}</strong></div><div class="subset"><span>其中推理 Token（输出子集）</span><strong>{{ formatCompact(summary?.tokens.reasoning ?? 0) }}</strong></div></div></article>
    </section>

    <section v-if="!sourceUnavailable" class="panel matrix-panel"><div class="panel-heading"><div><span class="eyebrow">核心分析</span><h2>来源 × 模型 × 推理强度</h2></div><span class="coverage"><i :class="{ partial: !summary?.pricing.complete }"></i>费用覆盖 {{ pricedPercent }}%</span></div><div class="table-scroll"><table><thead><tr><th>来源</th><th>模型 / Provider</th><th>推理强度</th><th>调用</th><th>速</th><th>首</th><th>量</th><th>费</th></tr></thead><tbody><tr v-for="row in stats" :key="`${row.sourceId}-${row.model}-${row.reasoningEffort}`"><td><span class="source-badge">{{ row.sourceName }}</span></td><td><strong>{{ row.model }}</strong><small>{{ row.provider }}</small></td><td><span class="effort-badge">{{ effortName(row.reasoningEffort) }}</span></td><td>{{ row.callCount }}<small>{{ row.performanceSampleCount }} 性能样本</small></td><td>{{ formatTps(row.averageEffectiveTps) }}</td><td>{{ formatDuration(row.averageTtftMs) }}</td><td>{{ formatCompact(row.tokens.total) }}</td><td><strong>{{ formatCost(row.estimatedCostNanoUsd, row.pricing.complete) }}</strong><small>{{ Math.round(row.pricing.ratio * 100) }}% 覆盖</small></td></tr><tr v-if="!stats.length"><td colspan="8"><div class="empty">当前筛选没有可用数据</div></td></tr></tbody></table></div></section>

    <section v-if="!sourceUnavailable" class="panel integrity-panel"><div class="panel-heading"><div><span class="eyebrow">统计可信度</span><h2>{{ integrity?.reconciled ? '已对账' : '存在统计缺口' }}</h2></div><span class="integrity-state" :class="{ warning: !integrity?.reconciled }">{{ integrity?.reconciled ? '原始层与统计层一致' : '数据尚未完全同步' }}</span></div><div class="table-scroll"><table><thead><tr><th>来源</th><th>原始调用</th><th>已索引</th><th>差异</th><th>未读</th><th>解析错误</th><th>最后调用</th><th>同步延迟</th></tr></thead><tbody><tr v-for="item in integrity?.sources" :key="item.sourceId"><td><strong>{{ item.sourceName }}</strong></td><td>{{ item.rawCallCount }}</td><td>{{ item.indexedCallCount }}</td><td :class="{ 'danger-text': item.difference }">{{ item.difference }}</td><td>{{ formatBytes(item.unreadBytes) }}</td><td :class="{ 'danger-text': item.parseErrorCount }">{{ item.parseErrorCount }}</td><td>{{ formatTime(item.lastCallAt) }}</td><td>{{ formatDuration(item.syncDelayMs) }}</td></tr></tbody></table></div></section>
    </template>

    <section v-else class="pricing-page"><div class="pricing-toolbar"><label class="search-box"><Search :size="16"/><input v-model="priceSearch" placeholder="搜索模型或 Vendor" /></label><button class="primary-button" @click="openPriceEditor()"><Plus :size="16"/>新增价格</button></div><div class="panel pricing-table"><div class="panel-heading"><div><span class="eyebrow">价格管理</span><h2>模型与生效版本</h2></div><span class="coverage">{{ pricingModels.length }} 个模型</span></div><div class="table-scroll"><table><thead><tr><th>模型 / Vendor</th><th>调用</th><th>输入</th><th>缓存读</th><th>缓存写</th><th>输出</th><th>生效区间</th><th>操作</th></tr></thead><tbody><template v-for="model in filteredPricingModels" :key="model.model"><tr v-if="!model.rates.length"><td><strong>{{ model.model }}</strong><small>{{ model.vendor }}</small></td><td>{{ model.callCount }}</td><td colspan="5"><span class="unpriced">未计价</span></td><td><button class="icon-button compact" title="为此模型创建价格" @click="openPriceEditor(undefined, model)"><Plus :size="15"/></button></td></tr><tr v-for="rate in model.rates" :key="rate.id"><td><strong>{{ rate.model }}</strong><small>{{ rate.vendor }} · {{ rate.aliases.join(', ') || '无别名' }}</small></td><td>{{ model.callCount }}</td><td>${{ rate.inputUsdPerMillion }}</td><td>{{ rate.cachedReadUsdPerMillion == null ? '空白' : `$${rate.cachedReadUsdPerMillion}` }}</td><td>{{ rate.cachedWriteUsdPerMillion == null ? '空白' : `$${rate.cachedWriteUsdPerMillion}` }}</td><td>${{ rate.outputUsdPerMillion }}</td><td>{{ rate.effectiveFrom }}<small>至 {{ rate.effectiveTo || '长期' }}</small></td><td><div class="row-actions"><button class="icon-button compact" title="编辑价格" @click="openPriceEditor(rate)"><Pencil :size="14"/></button><button class="icon-button compact danger-text" title="删除价格版本" @click="deletePrice(rate)"><Trash2 :size="14"/></button></div></td></tr></template></tbody></table></div></div></section>

    <div v-if="pricingEditor" class="modal-backdrop" @click.self="pricingEditor = false"><form class="price-editor" @submit.prevent="savePrice"><div class="panel-heading"><div><span class="eyebrow">价格版本</span><h2>{{ editingRateId == null ? '新增价格' : '编辑价格' }}</h2></div><button type="button" class="icon-button compact" aria-label="关闭价格编辑" @click="pricingEditor = false"><X :size="16"/></button></div><div class="form-grid"><label><span>Vendor</span><input v-model="priceForm.vendor" required /></label><label><span>模型</span><input v-model="priceForm.model" required /></label><label class="wide"><span>别名（逗号分隔）</span><input :value="priceForm.aliases.join(', ')" @input="priceForm.aliases = ($event.target as HTMLInputElement).value.split(',').map(v => v.trim()).filter(Boolean)" /></label><label><span>输入 $/百万</span><input v-model="priceForm.inputUsdPerMillion" inputmode="decimal" required /></label><label><span>缓存读取 $/百万</span><input v-model="priceForm.cachedReadUsdPerMillion" inputmode="decimal" placeholder="空白表示未计价" /></label><label><span>缓存写入 $/百万</span><input v-model="priceForm.cachedWriteUsdPerMillion" inputmode="decimal" placeholder="空白表示未计价" /></label><label><span>输出 $/百万</span><input v-model="priceForm.outputUsdPerMillion" inputmode="decimal" required /></label><label><span>生效日期</span><input v-model="priceForm.effectiveFrom" type="date" required /></label><label><span>结束日期</span><input v-model="priceForm.effectiveTo" type="date" /></label><label class="wide"><span>来源链接</span><input v-model="priceForm.sourceUrl" type="url" /></label></div><p class="form-hint">缓存价格空白表示该桶未计价；填写 0 表示免费。</p><div class="editor-actions"><button type="button" class="secondary-button" @click="pricingEditor = false">取消</button><button class="primary-button" type="submit"><Check :size="15"/>保存并重算</button></div></form></div>

  </main>

  <main v-else class="settings-page" aria-label="Agent Meter 设置"><header class="settings-header"><div class="brand"><div class="brand-mark"><Settings :size="20" /></div><div><strong>Agent Meter 设置</strong><span>菜单栏、数据源与应用更新</span></div></div><small>v{{ updateState.currentVersion }}</small></header>
    <div v-if="error" class="notice error"><CircleAlert :size="18" />{{ error }}<button aria-label="关闭错误" @click="error = ''"><X :size="16" /></button></div>
    <div v-if="lastAction" class="notice success"><Check :size="18" />{{ lastAction }}<button aria-label="关闭提示" @click="lastAction = ''"><X :size="16" /></button></div>
      <section><h3>菜单栏四指标</h3><p class="settings-note">每项固定为 30×22pt，大数值在上、指标名在下；应用图标默认隐藏。四项共用下方周期。</p><div class="menu-period segmented"><button v-for="[key, label] in periods" :key="key" :class="{ active: appSettings.menuPeriod === key }" @click="selectPeriod(key)">{{ label }}</button></div>
        <div v-for="item in ([['effectiveTps','速','平均有效 TPS',Zap],['ttft','首','平均首响时间',Timer],['todayTokens','量','Token 累加总量',Gauge],['estimatedCost','费','API 等价费用',Coins]] as const)" :key="item[0]" class="source-row"><div class="metric-mini">{{ item[1] }}</div><div><strong>{{ item[1] }} · {{ item[2] }}</strong><small>独立固定宽度状态项</small></div><button class="switch" :class="{ on: appSettings.menuMetrics[item[0]] }" role="switch" :aria-checked="appSettings.menuMetrics[item[0]]" :aria-label="`切换${item[1]}菜单栏指标`" @click="toggleMenuMetric(item[0])"><i></i></button></div>
        <div class="source-row"><div class="metric-mini">标</div><div><strong>应用图标</strong><small>关闭后由指标项承载菜单入口</small></div><button class="switch" :class="{ on: appSettings.showAppIcon }" role="switch" :aria-checked="appSettings.showAppIcon" aria-label="切换应用图标显示" @click="toggleAppIcon"><i></i></button></div>
      </section>
      <section><h3>本机只读数据源</h3><div v-for="source in sources" :key="source.id" class="source-row"><div class="source-icon"><Database :size="19" /></div><div><strong>{{ source.name }}</strong><small>{{ source.rootPath }} · {{ source.fileCount }} 个数据文件 · {{ formatBytes(source.totalBytes) }}</small><em v-if="source.limitation || source.error">{{ source.limitation || source.error }}</em></div><button class="switch" :class="{ on: source.enabled }" role="switch" :aria-checked="source.enabled" :aria-label="`${source.enabled ? '停用' : '启用'} ${source.name}`" @click="toggleSource(source)"><i></i></button></div></section>
      <section><h3>索引与同步</h3><div class="settings-card"><div class="card-heading"><div><strong>{{ importStatus?.message || '正在读取状态' }}</strong><small>{{ importStatus?.filesDone ?? 0 }} / {{ importStatus?.filesTotal ?? 0 }} 个文件</small></div></div><progress :value="importStatus?.bytesDone ?? 0" :max="importStatus?.bytesTotal || 1"></progress><div class="button-row"><button v-if="importStatus?.running" class="secondary-button" @click="pauseImport"><Pause :size="16"/>暂停</button><button v-else class="secondary-button" @click="runImport(false)"><Play :size="16"/>继续同步</button><button class="danger-button" @click="runImport(true)"><RefreshCw :size="16"/>重建 Codex 索引</button></div></div></section>
      <section><h3>API 等价价目</h3><div class="settings-card"><div class="card-heading"><div><strong>目录 {{ pricing?.version ?? '—' }}</strong><small>最后核验 {{ pricing?.verifiedAt ?? '—' }} · {{ pricing?.rates.length ?? 0 }} 个价格项</small></div><button class="secondary-button" @click="reprice"><Coins :size="15"/>重新计价</button></div><p class="settings-note pricing-note">按调用日期和官方标准文本 API 单价计算 USD 等价费用，不代表订阅套餐或实际账单。未知价格不会按零计入。</p><details><summary>查看价目与官方链接</summary><a v-for="rate in pricing?.rates" :key="`${rate.vendor}-${rate.model}`" :href="rate.sourceUrl" target="_blank" rel="noreferrer"><strong>{{ rate.model }}</strong><span>输入 ${{ rate.inputUsdPerMillion }} · 缓存读 {{ rate.cachedReadUsdPerMillion ?? '未公布' }} · 输出 ${{ rate.outputUsdPerMillion }} / 1M</span></a></details></div></section>
      <section><h3>应用</h3><div class="source-row"><div class="source-icon"><Play :size="19"/></div><div><strong>登录时启动</strong><small>后台启动时只显示菜单栏，不占用 Dock</small></div><button class="switch" :class="{ on: autostartEnabled }" role="switch" :aria-checked="autostartEnabled" aria-label="切换登录时启动" @click="toggleAutostart"><i></i></button></div></section>
      <section><h3>软件更新</h3><div class="source-row"><div class="source-icon"><CloudDownload :size="19"/></div><div><strong>自动检查稳定版</strong><small>启动后检查，之后每 24 小时检查</small></div><button class="switch" :class="{ on: appSettings.updates.automaticCheck }" role="switch" :aria-checked="appSettings.updates.automaticCheck" aria-label="切换自动检查更新" @click="toggleAutomaticUpdates"><i></i></button></div><div class="settings-card"><div class="card-heading"><div><strong>Agent Meter {{ updateState.currentVersion }}</strong><small>{{ appSettings.updates.lastCheckedAt ? `上次检查 ${formatTime(appSettings.updates.lastCheckedAt)}` : '尚未检查更新' }}</small></div><button class="secondary-button" :disabled="['checking','downloading'].includes(updateState.phase)" @click="checkForUpdates(false)"><LoaderCircle v-if="updateState.phase === 'checking'" :size="15" class="spin"/><RefreshCw v-else :size="15"/>检查更新</button></div><div v-if="updateState.phase === 'available'" class="update-result"><strong>发现 {{ updateState.version }}</strong><p>{{ updateState.notes }}</p><button class="primary-button" @click="downloadAndRestart"><CloudDownload :size="16"/>下载并重启</button></div><div v-else-if="updateState.phase === 'downloading' || updateState.phase === 'ready'" class="update-result"><span>{{ updateState.phase === 'ready' ? '安装完成，正在重启' : `正在下载并验证 ${Math.round(updateProgress)}%` }}</span><progress :value="updateState.downloadedBytes" :max="updateState.totalBytes || 1"></progress></div><div v-else-if="updateState.phase === 'current'" class="inline-result success-text"><Check :size="15"/>已是最新版</div><div v-else-if="updateState.phase === 'error'" class="inline-result error-text"><CircleAlert :size="15"/>{{ updateState.error }}</div></div></section>
      <section class="privacy-note"><Info :size="18"/><p><strong>数据只留在本机</strong><span>只读数值指标，不保存对话、提示词、工具参数或工具输出。</span></p></section>
  </main>
</template>
