import { expect, it } from 'vitest'

import type { AttentionItem } from '@/lib/api/attentionApi'

import { activityDots } from '../ActivityBar'

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
