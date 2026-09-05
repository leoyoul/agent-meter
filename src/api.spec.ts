import { describe, expect, it } from 'vitest'
import { meterApi } from './api'

describe('browser mock API', () => {
  it('returns complete dashboard data without a Tauri runtime', async () => {
    expect(meterApi.isTauri()).toBe(false)
    const [overview, series, models, tasks] = await Promise.all([
      meterApi.queryOverview({ agentKind: 'all' }),
      meterApi.queryTimeseries({}),
      meterApi.queryModelStats({}),
      meterApi.queryTasks({}),
    ])

    expect(overview.tokens.total).toBeGreaterThan(0)
    expect(series).toHaveLength(14)
    expect(models.length).toBeGreaterThanOrEqual(3)
    expect(tasks.some(task => task.agentKind === 'subagent')).toBe(true)
  })

  it('applies task filters and updates source state', async () => {
    const fastTasks = await meterApi.queryTasks({ model: 'codex-fast', agentKind: 'subagent' })
    expect(fastTasks).toHaveLength(1)
    expect(fastTasks[0].agentKind).toBe('subagent')

    const [source] = await meterApi.discoverSources()
    const updated = await meterApi.updateSource(source.id, false)
    expect(updated.enabled).toBe(false)
    await meterApi.updateSource(source.id, true)
  })

  it('supports import lifecycle actions', async () => {
    const started = await meterApi.startImport(true)
    expect(started.running).toBe(true)
    expect(started.message).toContain('重建')

    const paused = await meterApi.pauseImport()
    expect(paused.paused).toBe(true)
    expect(paused.running).toBe(false)
  })
})
