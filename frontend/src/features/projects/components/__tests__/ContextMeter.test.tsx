import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ContextMeter, latestContextUsage } from '@/features/projects/components/ContextMeter'
import type { ConversationTurnResponse } from '@/lib/workspaceClient'

const turn = (id: string, role: 'user' | 'assistant', tokenUsage: Record<string, unknown> | null = null) => ({
    id, role, kind: 'message', status: 'complete', content: '', timestamp: '2026-10-15T10:00:00Z', token_usage: tokenUsage,
}) as ConversationTurnResponse

describe('ContextMeter', () => {
    it('shows the latest assistant turn context and hides without data', () => {
        const usage = latestContextUsage([
            turn('a1', 'assistant', { context: { used_tokens: 900_000, window_tokens: 1_000_000 } }),
            turn('a2', 'assistant', { input_tokens: 5, context: { used_tokens: 24_000, window_tokens: 1_000_000 } }),
            turn('u3', 'user'),
        ])
        const { rerender } = render(<ContextMeter usage={usage} />)
        expect(screen.getByTestId('chat-context-meter')).toHaveTextContent('Context 24k / 1M · 2%')

        rerender(<ContextMeter usage={latestContextUsage([turn('a1', 'assistant', { context: { window_tokens: 200_000 } })])} />)
        expect(screen.getByTestId('chat-context-meter')).toHaveTextContent('Context window 200k')

        rerender(<ContextMeter usage={latestContextUsage([turn('a1', 'assistant', { input_tokens: 5 }), turn('u2', 'user')])} />)
        expect(screen.queryByTestId('chat-context-meter')).toBeNull()

        rerender(<ContextMeter usage={latestContextUsage([
            turn('a1', 'assistant', { context: { used_tokens: 900_000, window_tokens: 1_000_000 } }),
            turn('a2', 'assistant', { input_tokens: 5 }),
        ])} />)
        expect(screen.queryByTestId('chat-context-meter')).toBeNull()
    })
})
