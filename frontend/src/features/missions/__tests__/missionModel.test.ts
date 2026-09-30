import { expect, it } from 'vitest'
import type { RunRecord } from '@/features/runs/model/shared'
import type { Mission } from '../MissionsPanel'
import { joinRuns, missionTime, parseTurn, shortLine, totalTokens } from '../model/missionModel'

const now = Date.parse('2026-09-28T12:30:00Z')
const base: Mission = { id: 'm', revision: 1, fields: { title: 'T', description: '', archived: false }, activity: [], created_at: '2026-09-27 11:48:00.340499 +00:00:00' }
const roster = (run_id: string, status: string, summary = '') => ({ run_id, flow_name: 'work/build-it.yaml', summary, launched_at: '2026-09-28 12:18:00.5 +00:00:00', status })

it('gives each state one short line, and a closed mission never its reason', () => {
    expect(shortLine({ ...base, status: 'draft' }, now)).toBe('Draft · created Sep 27')
    expect(shortLine({ ...base, status: 'running' }, now)).toBe('Agent is working')
    expect(shortLine({ ...base, status: 'running', runs: [roster('r0', 'completed'), roster('r1', 'running', 'Build the index')] }, now)).toBe('Build the index · 12m')
    expect(shortLine({ ...base, status: 'running', runs: [roster('r1', 'running')] }, now)).toBe('Build It · 12m')
    expect(shortLine({ ...base, status: 'running', waiting: true, wait_reason: 'Review pending' }, now)).toBe('Waiting: Review pending')
    expect(shortLine({ ...base, status: 'needs_you' }, now)).toBe('Waiting for your reply')
    expect(shortLine({ ...base, status: 'needs_you', question: { prompt: 'Ship it?', options: [{ label: 'Ship it' }] } }, now)).toBe('Ship it?')
    for (const status of ['done', 'failed', 'canceled'] as const) {
        const line = shortLine({ ...base, status: 'closed', closed: { status, reason: 'Six lines of closing summary.\nMore.', at: '2026-09-27 09:00:00.1 +00:00:00' } }, now)
        expect(line).toBe(`${{ done: 'Done', failed: 'Failed', canceled: 'Canceled' }[status]} · Sep 27`)
        expect(line).not.toContain('summary')
    }
})

it('reads stored timestamps whose hour has one digit', () => {
    expect(missionTime('2026-09-28 2:18:19.591335 +00:00:00')).toBe('2026-09-28T02:18:19Z')
    expect(missionTime('2026-09-28 12:18:19.5 +00:00:00')).toBe('2026-09-28T12:18:19Z')
    const closed = shortLine({ ...base, status: 'closed', closed: { status: 'done', reason: '', at: '2026-09-28 2:18:19.591335 +00:00:00' } }, now)
    expect(closed).toMatch(/^Done · \S/)
    expect(shortLine({ ...base, status: 'running', runs: [{ ...roster('r1', 'running', 'Early'), launched_at: '2026-09-28 9:30:00.1 +00:00:00' }] }, now)).toBe('Early · 3h 0m')
})

it('reads the start, run results, run questions and replies from user turns', () => {
    expect(parseTurn('Objective:\nShip search.\n\nKeep it small.\n\nBegin work on this mission.')).toEqual([{ kind: 'start' }])
    const question = { flow_name: 'Question Child (test)', options: [{ key: 'B', label: 'Blue', value: 'Blue' }, 'Green'], prompt: 'Which color?', question_id: 'q', root_run_id: 'run-root', run_id: 'child' }
    expect(parseTurn([
        'Run run-a (testing/question-parent.yaml, "Test \\"quoted\\" handling") ended completed.',
        'Run run-b (work/build.yaml, "Build") ended failed: boom\n\nstill the error.',
        'Run run-c (work/build.yaml, "") is waiting on a recovery decision.',
        `Run question: ${JSON.stringify(question)}`,
        'User: ship it\n\nplease',
        'Trigger fired: "hook" (webhook)',
    ].join('\n\n'))).toEqual([
        { kind: 'result', runId: 'run-a', flowName: 'testing/question-parent.yaml', summary: 'Test "quoted" handling', status: 'completed', error: '' },
        { kind: 'result', runId: 'run-b', flowName: 'work/build.yaml', summary: 'Build', status: 'failed', error: 'boom\n\nstill the error' },
        { kind: 'result', runId: 'run-c', flowName: 'work/build.yaml', summary: '', status: 'waiting', error: '' },
        { kind: 'question', prompt: 'Which color?', options: ['Blue', 'Green'], runId: 'run-root', flowName: 'Question Child (test)' },
        { kind: 'reply', text: 'ship it\n\nplease' },
        { kind: 'text', text: 'Trigger fired: "hook" (webhook)' },
    ])
    expect(parseTurn('Run question: not json')).toEqual([{ kind: 'text', text: 'Run question: not json' }])
})

it('joins the roster with run facts by run id', () => {
    const record = (run: Partial<RunRecord>) => ({ flow_name: 'work/build-it.yaml', status: 'completed', working_directory: '', model: '', started_at: '2026-09-28T12:00:00Z', ...run }) as RunRecord
    const runs = joinRuns(
        [roster('r1', 'completed', 'Build'), roster('r2', 'waiting', 'Review'), roster('r3', 'running', 'Unknown to the list')],
        [record({ run_id: 'r1', title: 'Build the search index', ended_at: '2026-09-28T12:04:30Z', token_usage: 1200 }), record({ run_id: 'r2', status: 'running', token_usage: 800 }), record({ run_id: 'other', token_usage: 5 })],
        now,
    )
    expect(runs).toEqual([
        { runId: 'r1', title: 'Build the search index', flowTitle: 'Build It', status: 'completed', kind: 'completed', duration: '4m', tokens: 1200 },
        { runId: 'r2', title: 'Review', flowTitle: 'Build It', status: 'waiting', kind: 'waiting', duration: '30m', tokens: 800 },
        { runId: 'r3', title: 'Unknown to the list', flowTitle: 'Build It', status: 'running', kind: 'running', duration: '12m', tokens: null },
    ])
    expect(totalTokens(runs)).toBe(2000)
    expect(totalTokens(runs.slice(2))).toBeNull()
})
