<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Activity, Bot, Check, ChevronDown, CircleAlert, Database, Gauge, LoaderCircle, Pause, Play, RefreshCw, Settings, Sparkles, Users, X } from 'lucide-vue-next'
import { meterApi } from './api'
import type { AgentKind, ImportStatus, MetricFilters, ModelStat, Overview, SourceInfo, TaskRow, TimeseriesPoint } from './shared'

const localDate = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
const now = new Date()
const before = new Date(now); before.setDate(before.getDate() - 13)
const filters = ref<MetricFilters>({ startDate: localDate(before), endDate: localDate(now), agentKind: 'all' })
const overview = ref<Overview | null>(null)
const series = ref<TimeseriesPoint[]>([])
const models = ref<ModelStat[]>([])
const tasks = ref<TaskRow[]>([])
const sources = ref<SourceInfo[]>([])
const importStatus = ref<ImportStatus | null>(null)
const loading = ref(true)
const error = ref('')
const settingsOpen = ref(false)
const autostartEnabled = ref(false)
const lastAction = ref('')
const unlisteners: Array<() => void> = []

const projects = computed(() => [...new Set(tasks.value.map(task => task.project).filter(Boolean))].sort())
const modelNames = computed(() => [...new Set(models.value.map(item => item.model))].sort())
const sourceSelected = computed(() => sources.value.find(source => source.id === filters.value.sourceId)?.name ?? '全部数据源')
const maxTokens = computed(() => Math.max(...series.value.map(point => point.totalTokens), 1))
const chartPoints = computed(() => series.value.map((point, index) => {
  const x = series.value.length <= 1 ? 0 : index / (series.value.length - 1) * 100
  const y = 94 - point.totalTokens / maxTokens.value * 82
  return `${x},${y}`
}).join(' '))
const chartArea = computed(() => `0,100 ${chartPoints.value} 100,100`)
const rootTasks = computed(() => tasks.value.filter(task => task.agentKind === 'root'))

const formatCompact = (value: number) => new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
const formatDuration = (value: number | null) => value == null ? '—' : value < 1000 ? `${Math.round(value)} ms` : value < 60_000 ? `${(value / 1000).toFixed(1)} s` : `${(value / 60_000).toFixed(1)} min`
const formatTps = (value: number | null) => value == null ? '—' : value.toFixed(1)
const formatTime = (value: string | null) => value ? new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(new Date(value)) : '—'
const formatBytes = (value: number) => value >= 1_073_741_824 ? `${(value / 1_073_741_824).toFixed(1)} GB` : `${Math.round(value / 1_048_576)} MB`
const childCount = (sessionId: string) => tasks.value.filter(task => task.parentThreadId === sessionId).length

async function loadAll(silent = false) {
  if (!silent) loading.value = true
  error.value = ''
  try {
    const f = { ...filters.value }
    const [nextOverview, nextSeries, nextModels, nextTasks, nextStatus] = await Promise.all([
      meterApi.queryOverview(f), meterApi.queryTimeseries(f), meterApi.queryModelStats(f), meterApi.queryTasks(f), meterApi.getImportStatus(),
    ])
    overview.value = nextOverview; series.value = nextSeries; models.value = nextModels; tasks.value = nextTasks; importStatus.value = nextStatus
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally { loading.value = false }
}

async function toggleSource(source: SourceInfo) {
  try {
    const updated = await meterApi.updateSource(source.id, !source.enabled)
    sources.value = sources.value.map(item => item.id === updated.id ? updated : item)
    lastAction.value = `${updated.name} 已${updated.enabled ? '启用' : '停用'}`
    await loadAll(true)
  } catch (reason) { error.value = String(reason) }
}

async function runImport(force = false) {
  try { importStatus.value = await meterApi.startImport(force); lastAction.value = force ? '已开始重建索引' : '已开始刷新' }
  catch (reason) { error.value = String(reason) }
}

async function pauseImport() {
  try { importStatus.value = await meterApi.pauseImport(); lastAction.value = '导入已暂停' }
  catch (reason) { error.value = String(reason) }
}

async function toggleAutostart() {
  try {
    autostartEnabled.value = await meterApi.setAutostartEnabled(!autostartEnabled.value)
    lastAction.value = `登录时启动已${autostartEnabled.value ? '开启' : '关闭'}`
  } catch (reason) { error.value = String(reason) }
}

watch(filters, () => loadAll(), { deep: true })

onMounted(async () => {
  sources.value = await meterApi.discoverSources()
  autostartEnabled.value = await meterApi.isAutostartEnabled()
  await loadAll()
  unlisteners.push(await meterApi.on<ImportStatus>('import-progress', payload => { importStatus.value = payload }))
  unlisteners.push(await meterApi.on('metrics-updated', () => loadAll(true)))
  unlisteners.push(await meterApi.on<string>('source-error', payload => { error.value = payload }))
})
onBeforeUnmount(() => unlisteners.splice(0).forEach(fn => fn()))
</script>

<template>
  <main class="app-shell">
    <header class="topbar">
      <div class="brand">
        <div class="brand-mark"><Activity :size="20" /></div>
        <div><strong>Agent Meter</strong><span>本机 Agent 运行仪表盘</span></div>
      </div>
      <div class="header-actions">
        <span class="freshness"><i :class="{ live: importStatus?.running }"></i>{{ importStatus?.running ? '正在索引' : `更新于 ${formatTime(overview?.lastUpdatedAt ?? null)}` }}</span>
        <button class="icon-button" title="刷新数据" aria-label="刷新数据" @click="runImport(false)"><RefreshCw :size="18" :class="{ spin: importStatus?.running }" /></button>
        <button class="icon-button" title="数据源设置" aria-label="打开数据源设置" @click="settingsOpen = true"><Settings :size="18" /></button>
      </div>
    </header>

    <section class="filters glass-panel" aria-label="统计筛选">
      <label><span>日期范围</span><div class="date-range"><input v-model="filters.startDate" type="date"><span>至</span><input v-model="filters.endDate" type="date"></div></label>
      <label><span>数据源</span><div class="select-wrap"><select v-model="filters.sourceId"><option :value="undefined">全部数据源</option><option v-for="source in sources" :key="source.id" :value="source.id">{{ source.name }}</option></select><ChevronDown :size="15" /></div></label>
      <label><span>模型</span><div class="select-wrap"><select v-model="filters.model"><option value="">全部模型</option><option v-for="name in modelNames" :key="name">{{ name }}</option></select><ChevronDown :size="15" /></div></label>
      <label><span>项目</span><div class="select-wrap"><select v-model="filters.project"><option value="">全部项目</option><option v-for="project in projects" :key="project">{{ project }}</option></select><ChevronDown :size="15" /></div></label>
      <fieldset><legend>Agent 类型</legend><div class="segmented"><button v-for="item in ([['all','全部'],['root','主代理'],['subagent','子代理']] as [AgentKind,string][] )" :key="item[0]" :class="{ active: filters.agentKind === item[0] }" @click="filters.agentKind = item[0]">{{ item[1] }}</button></div></fieldset>
    </section>

    <div v-if="error" class="notice error"><CircleAlert :size="18" />{{ error }}<button aria-label="关闭错误" @click="error = ''"><X :size="16" /></button></div>
    <div v-if="lastAction" class="notice success"><Check :size="18" />{{ lastAction }}<button aria-label="关闭提示" @click="lastAction = ''"><X :size="16" /></button></div>

    <section class="kpi-grid" aria-label="今日总览">
      <article class="kpi primary"><div class="kpi-icon"><Sparkles :size="19" /></div><span>Token 总量</span><strong>{{ overview ? formatCompact(overview.tokens.total) : '—' }}</strong><small>输入 {{ formatCompact(overview?.tokens.input ?? 0) }} · 输出 {{ formatCompact(overview?.tokens.output ?? 0) }}</small></article>
      <article class="kpi"><div class="kpi-icon amber"><Gauge :size="19" /></div><span>近 5 轮首响</span><strong>{{ formatDuration(overview?.recentMedianTtftMs ?? null) }}</strong><small>P95 {{ formatDuration(overview?.p95TtftMs ?? null) }}</small></article>
      <article class="kpi"><div class="kpi-icon green"><Activity :size="19" /></div><span class="with-tip">近 5 轮有效 TPS <i title="输出 Token ÷（整轮耗时 - 首响时间）。包含思考、工具等待和多次模型调用。">?</i></span><strong>{{ formatTps(overview?.recentMedianEffectiveTps ?? null) }}</strong><small>全期中位数 {{ formatTps(overview?.medianEffectiveTps ?? null) }}</small></article>
      <article class="kpi"><div class="kpi-icon pink"><Users :size="19" /></div><span>任务轮次</span><strong>{{ overview?.turnCount ?? '—' }}</strong><small>{{ overview?.subagentCount ?? 0 }} 个子代理 · {{ overview?.runningCount ?? 0 }} 个运行中</small></article>
    </section>

    <section class="content-grid">
      <article class="panel chart-panel">
        <div class="panel-heading"><div><span class="eyebrow">TOKEN 趋势</span><h2>近 14 天用量</h2></div><div class="legend"><span><i class="dot indigo"></i>总 Token</span><span><i class="dot cyan"></i>输出</span></div></div>
        <div v-if="loading" class="loading"><LoaderCircle class="spin" :size="22" />正在读取本机索引</div>
        <div v-else class="chart-wrap">
          <div class="axis"><span>{{ formatCompact(maxTokens) }}</span><span>{{ formatCompact(maxTokens / 2) }}</span><span>0</span></div>
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" role="img" aria-label="Token 用量趋势折线图">
            <defs><linearGradient id="area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".32"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs>
            <line v-for="y in [12,53,94]" :key="y" x1="0" :y1="y" x2="100" :y2="y" class="gridline" />
            <polygon :points="chartArea" fill="url(#area)" />
            <polyline :points="chartPoints" class="chart-line" />
          </svg>
          <div class="chart-labels"><span v-for="point in series.filter((_, i) => i % 3 === 0 || i === series.length - 1)" :key="point.bucket">{{ point.bucket.slice(5) }}</span></div>
        </div>
      </article>

      <article class="panel breakdown-panel">
        <div class="panel-heading"><div><span class="eyebrow">构成</span><h2>Token 明细</h2></div><span class="source-pill">{{ sourceSelected }}</span></div>
        <div class="token-ring" :style="{ '--cached': `${overview ? overview.tokens.cachedInput / overview.tokens.total * 100 : 0}%`, '--input': `${overview ? overview.tokens.input / overview.tokens.total * 100 : 0}%`, '--output': `${overview ? (overview.tokens.input + overview.tokens.output) / overview.tokens.total * 100 : 0}%` }"><div><strong>{{ overview ? formatCompact(overview.tokens.total) : '—' }}</strong><span>总计</span></div></div>
        <div class="breakdown-list">
          <div><span><i class="dot indigo"></i>输入</span><strong>{{ formatCompact(overview?.tokens.input ?? 0) }}</strong></div>
          <div><span><i class="dot cyan"></i>缓存输入</span><strong>{{ formatCompact(overview?.tokens.cachedInput ?? 0) }}</strong></div>
          <div><span><i class="dot green"></i>输出</span><strong>{{ formatCompact(overview?.tokens.output ?? 0) }}</strong></div>
          <div><span><i class="dot pink"></i>推理</span><strong>{{ formatCompact(overview?.tokens.reasoning ?? 0) }}</strong></div>
        </div>
        <p class="footnote">缓存输入是输入 Token 的子集，不重复计入总量。</p>
      </article>
    </section>

    <section class="panel table-panel">
      <div class="panel-heading"><div><span class="eyebrow">效率对比</span><h2>模型表现</h2></div><span class="table-note">P50 / P95</span></div>
      <div class="table-scroll"><table><thead><tr><th>模型</th><th>任务</th><th>Token</th><th>首响</th><th>整轮耗时</th><th><span class="with-tip">有效 TPS <i title="包含思考、工具等待和多次模型调用，不代表纯模型解码速度。">?</i></span></th></tr></thead><tbody><tr v-for="row in models" :key="`${row.model}-${row.reasoningEffort}`"><td><strong>{{ row.model }}</strong><small>{{ row.reasoningEffort || '默认推理' }}</small></td><td>{{ row.turnCount }}</td><td>{{ formatCompact(row.tokens.total) }}</td><td>{{ formatDuration(row.medianTtftMs) }}<small>{{ formatDuration(row.p95TtftMs) }}</small></td><td>{{ formatDuration(row.medianDurationMs) }}<small>{{ formatDuration(row.p95DurationMs) }}</small></td><td><strong>{{ formatTps(row.medianEffectiveTps) }}</strong></td></tr></tbody></table></div>
    </section>

    <section class="lower-grid">
      <article class="panel task-tree">
        <div class="panel-heading"><div><span class="eyebrow">AGENT 结构</span><h2>主代理与子代理</h2></div><Bot :size="20" /></div>
        <div class="tree-list">
          <div v-for="root in rootTasks" :key="root.turnId" class="tree-group">
            <div class="tree-row"><div class="agent-avatar"><Bot :size="17" /></div><div><strong>{{ root.project }}</strong><small>{{ root.model }} · {{ formatTime(root.startedAt) }}</small></div><span>{{ childCount(root.sessionId) }} 个子代理</span></div>
            <div v-for="child in tasks.filter(task => task.parentThreadId === root.sessionId)" :key="child.turnId" class="tree-row child"><div class="branch">└</div><div class="agent-avatar sub"><Sparkles :size="15" /></div><div><strong>{{ child.agentPath?.split('/').pop() }}</strong><small>{{ child.model }} · {{ child.status === 'running' ? '运行中' : formatDuration(child.durationMs) }}</small></div><i class="status-dot" :class="child.status"></i></div>
          </div>
          <div v-if="!rootTasks.length" class="empty">当前筛选下没有主代理任务</div>
        </div>
      </article>

      <article class="panel recent-tasks">
        <div class="panel-heading"><div><span class="eyebrow">最近活动</span><h2>任务明细</h2></div><span>{{ tasks.length }} 条</span></div>
        <div class="activity-list"><div v-for="task in tasks" :key="task.turnId" class="activity-row"><i class="status-dot" :class="task.status"></i><div><strong>{{ task.project || '未命名项目' }}</strong><small>{{ task.agentKind === 'root' ? '主代理' : '子代理' }} · {{ task.model }}</small></div><div class="activity-metric"><strong>{{ formatCompact(task.tokens.total) }}</strong><small>{{ formatDuration(task.durationMs) }}</small></div></div><div v-if="!tasks.length" class="empty">当前筛选下没有任务</div></div>
      </article>
    </section>

    <div v-if="settingsOpen" class="drawer-backdrop" @click.self="settingsOpen = false">
      <aside class="settings-drawer" aria-label="数据源设置">
        <header><div><span class="eyebrow">设置</span><h2>数据与索引</h2></div><button class="icon-button" aria-label="关闭设置" title="关闭" @click="settingsOpen = false"><X :size="19" /></button></header>
        <section><h3>本机数据源</h3><div v-for="source in sources" :key="source.id" class="source-row"><div class="source-icon"><Database :size="19" /></div><div><strong>{{ source.name }}</strong><small>{{ source.rootPath }} · {{ source.fileCount }} 个文件 · {{ formatBytes(source.totalBytes) }}</small><em v-if="source.error">{{ source.error }}</em></div><button class="switch" :class="{ on: source.enabled }" role="switch" :aria-checked="source.enabled" :aria-label="`${source.enabled ? '停用' : '启用'} ${source.name}`" @click="toggleSource(source)"><i></i></button></div></section>
        <section><h3>索引状态</h3><div class="import-card"><div><strong>{{ importStatus?.message || '正在读取状态' }}</strong><span>{{ importStatus?.filesDone ?? 0 }} / {{ importStatus?.filesTotal ?? 0 }} 个文件</span></div><progress :value="importStatus?.bytesDone ?? 0" :max="importStatus?.bytesTotal || 1"></progress><small v-if="importStatus?.currentFile">{{ importStatus.currentFile }}</small><div class="import-actions"><button v-if="importStatus?.running" class="secondary-button" @click="pauseImport"><Pause :size="16" />暂停</button><button v-else class="secondary-button" @click="runImport(false)"><Play :size="16" />继续扫描</button><button class="danger-button" @click="runImport(true)"><RefreshCw :size="16" />重建索引</button></div></div></section>
        <section><h3>应用</h3><div class="source-row"><div class="source-icon"><Play :size="19" /></div><div><strong>登录时启动</strong><small>默认关闭，可随时在这里启用。</small></div><button class="switch" :class="{ on: autostartEnabled }" role="switch" :aria-checked="autostartEnabled" aria-label="切换登录时启动" @click="toggleAutostart"><i></i></button></div></section>
        <section class="privacy-note"><Database :size="18" /><p><strong>数据始终留在本机</strong><span>只读取数值指标，不保存提示词、回复正文、工具参数或工具输出。</span></p></section>
      </aside>
    </div>
  </main>
</template>
