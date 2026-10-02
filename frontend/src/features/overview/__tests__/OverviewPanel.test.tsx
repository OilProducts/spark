import { act, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AttentionItem } from '@/lib/api/attentionApi'
import { OverviewPanel } from '../OverviewPanel'

let pending: AttentionItem[] = []
vi.mock('@/lib/api/attentionApi', () => ({ fetchPendingAttention: () => Promise.resolve(pending) }))

// Run gate ids as the aggregate emits them: the run id with its run-local question id.
const gate = (runId: string, questionId: string): AttentionItem => ({
    kind: 'run_gate', id: `${runId}:${questionId}`, title: runId, project_path: '/work/app', run_id: runId, updated_at: '2026-10-02T09:00:00Z',
})
const rows = () => screen.queryAllByTestId('overview-needs-you-item').map((row) => row.querySelector('.truncate.text-sm')?.textContent)
const poll = async (next: AttentionItem[], expected: string[]) => {
    pending = next
    act(() => { window.dispatchEvent(new Event('focus')) })
    await waitFor(() => expect(rows()).toEqual(expected))
}

describe('Overview needs you', () => {
    it('keeps two runs at the same gate apart as one advances and resolves', async () => {
        pending = [gate('run-a', 'gate-1'), gate('run-b', 'gate-1')]
        render(<OverviewPanel />)
        await waitFor(() => expect(rows()).toEqual(['run-a', 'run-b']))

        await poll([gate('run-a', 'gate-2'), gate('run-b', 'gate-1')], ['run-a', 'run-b'])
        await poll([gate('run-b', 'gate-1')], ['run-b'])
    })
})
