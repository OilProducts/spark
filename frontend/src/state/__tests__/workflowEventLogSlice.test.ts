import { describe, expect, it, beforeEach } from 'vitest'

import { useStore } from '@/store'
import type { WorkflowLogEntry } from '@/state/workflowEventLogSlice'

const entry = (id: string, seq: number, overrides: Partial<WorkflowLogEntry> = {}): WorkflowLogEntry => ({
    id,
    seq,
    timestamp: '2026-03-24T12:00:00Z',
    kind: 'run_started',
    message: `event ${id}`,
    project_path: '/tmp/project',
    run_id: id.split(':')[0] ?? id,
    flow_name: 'ops/run.dot',
    ...overrides,
})

describe('workflowEventLogSlice', () => {
    beforeEach(() => {
        useStore.setState({ workflowEventLog: [] })
    })

    it('merges entries by id and orders them by sequence', () => {
        const apply = useStore.getState().applyWorkflowLogEntries
        apply([entry('run-b:run_started', 3)])
        apply([entry('run-a:run_started', 1)])
        // Replays of the same id replace rather than duplicate.
        apply([entry('run-a:run_started', 1, { message: 'replayed' })])

        const log = useStore.getState().workflowEventLog
        expect(log.map((item) => item.seq)).toEqual([1, 3])
        expect(log[0].message).toBe('replayed')
    })

    it('caps retained entries at 200, keeping the newest', () => {
        const apply = useStore.getState().applyWorkflowLogEntries
        apply(Array.from({ length: 250 }, (_, index) => entry(`run-${index}:run_started`, index)))

        const log = useStore.getState().workflowEventLog
        expect(log).toHaveLength(200)
        expect(log[0].seq).toBe(50)
        expect(log[log.length - 1].seq).toBe(249)
    })
})
