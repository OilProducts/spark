import { expect, it } from 'vitest'

import type { AttentionItem } from '@/lib/api/attentionApi'

import { activityDots } from '../ActivityBar'
import { attentionTimes } from '../useAttention'
import { formatRunDate, formatRunTime } from '@/features/runs/model/runOverviewModel'

const item = (kind: AttentionItem['kind']): AttentionItem => ({ kind, id: kind, title: '', project_path: '/p', updated_at: '' })

it('marks a view waiting when something there needs you, else running while work is in flight', () => {
    expect(activityDots([], [], [])).toEqual({ home: null, missions: null, runs: null })
    expect(activityDots([], ['completed', 'running'], ['running'])).toEqual({ home: null, missions: 'running', runs: 'running' })
    expect(activityDots([], ['queued'], ['draft', 'closed'])).toMatchObject({ missions: null, runs: 'running' })
    // Waiting outranks running.
    expect(activityDots([item('run_gate')], ['running'], ['running', 'needs_you'])).toEqual({ home: null, missions: 'waiting', runs: 'waiting' })
    expect(activityDots([item('proposed_plan')], [], [])).toMatchObject({ home: 'waiting' })
    expect(activityDots([], [], [], true)).toMatchObject({ home: 'running' })
    expect(activityDots([item('proposed_plan')], [], [], true)).toMatchObject({ home: 'waiting' })
    expect(activityDots([item('flow_run_request'), item('mission')], ['waiting'], [])).toEqual({ home: 'waiting', missions: 'waiting', runs: 'waiting' })
})

it('times repeated items of one title that share a minute to the second, and others to the minute', () => {
    const now = new Date(2026, 10, 12, 9, 0).getTime()
    const at = (minute: number, second: number) => new Date(2026, 10, 12, 0, minute, second).toISOString()
    const gate = (id: string, title: string, updated_at: string): AttentionItem => ({ kind: 'run_gate', id, title, project_path: '/p', updated_at })
    const items = [gate('a', 'Gate sweep', at(52, 7)), gate('b', 'Gate sweep', at(52, 31)), gate('c', 'Release', at(52, 40)), gate('d', 'Gate sweep', at(53, 0))]
    const timeOf = attentionTimes(items, [], now)
    expect(timeOf(items[0])).not.toBe(timeOf(items[1]))
    expect(timeOf(items[0])).toBe(formatRunTime(items[0].updated_at, true))
    expect(timeOf(items[2])).toBe(formatRunDate(items[2].updated_at, now))
    expect(timeOf(items[3])).toBe(formatRunDate(items[3].updated_at, now))
})
