import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { RunResultCard } from '../RunResultCard'
import type { PipelineResultResponse } from '@/lib/attractorClient'

const makeResult = (overrides: Partial<PipelineResultResponse> = {}): PipelineResultResponse => ({
    run_id: 'run-1',
    status: 'completed',
    state: 'ready',
    source_node_id: 'answer',
    source_artifact_path: 'logs/answer/response.md',
    display_mode: 'raw',
    body_markdown: 'Raw result',
    summary_enabled: false,
    summary_prompt: null,
    summary_error: null,
    error: null,
    ...overrides,
})

describe('RunResultCard', () => {
    it('renders pending and unavailable result states', () => {
        const { rerender } = render(
            <RunResultCard
                result={makeResult({ state: 'pending', body_markdown: '', source_artifact_path: null })}
                resultError={null}
                isLoading={false}
                onRefresh={vi.fn()}
                onViewSource={vi.fn()}
            />,
        )

        expect(screen.getByTestId('run-result-pending')).toHaveTextContent('Result will be available')

        rerender(
            <RunResultCard
                result={makeResult({ state: 'unavailable', body_markdown: '', source_artifact_path: null })}
                resultError={null}
                isLoading={false}
                onRefresh={vi.fn()}
                onViewSource={vi.fn()}
            />,
        )

        expect(screen.getByTestId('run-result-unavailable')).toHaveTextContent('No result source')
    })

    it('renders raw and summarized result bodies with source access', () => {
        const onViewSource = vi.fn()
        const { rerender } = render(
            <RunResultCard
                result={makeResult({ body_markdown: 'Raw **result**', display_mode: 'raw' })}
                resultError={null}
                isLoading={false}
                onRefresh={vi.fn()}
                onViewSource={onViewSource}
            />,
        )

        expect(screen.getByTestId('run-result-body')).toHaveTextContent('Raw result')
        fireEvent.click(screen.getByTestId('run-result-source-button'))
        expect(onViewSource).toHaveBeenCalledWith('logs/answer/response.md')

        rerender(
            <RunResultCard
                result={makeResult({
                    body_markdown: 'Summary result',
                    display_mode: 'summary',
                    summary_enabled: true,
                })}
                resultError={null}
                isLoading={false}
                onRefresh={vi.fn()}
                onViewSource={vi.fn()}
            />,
        )

        expect(screen.getByTestId('run-result-body')).toHaveTextContent('Summary result')
    })
})
