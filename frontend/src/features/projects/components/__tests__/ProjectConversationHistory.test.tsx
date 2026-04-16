import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ProjectConversationHistory } from '@/features/projects/components/ProjectConversationHistory'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'

const formatConversationTimestamp = (value: string) => `formatted:${value}`

const renderHistory = (activeConversationHistory: ConversationTimelineEntry[]) =>
    render(
        <ProjectConversationHistory
            activeConversationId="conversation-1"
            isConversationHistoryLoading={false}
            hasRenderableConversationHistory={activeConversationHistory.length > 0}
            activeConversationHistory={activeConversationHistory}
            activeFlowRunRequestsById={new Map()}
            activeFlowLaunchesById={new Map()}
            latestFlowRunRequestId={null}
            latestFlowLaunchId={null}
            expandedToolCalls={{}}
            expandedThinkingEntries={{}}
            pendingFlowRunRequestId={null}
            formatConversationTimestamp={formatConversationTimestamp}
            onToggleToolCallExpanded={vi.fn()}
            onToggleThinkingEntryExpanded={vi.fn()}
            onReviewFlowRunRequest={vi.fn()}
            onOpenFlowRun={vi.fn()}
        />,
    )

const InteractiveHistory = ({
    activeConversationHistory,
}: {
    activeConversationHistory: ConversationTimelineEntry[]
}) => {
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinkingEntries, setExpandedThinkingEntries] = useState<Record<string, boolean>>({})

    return (
        <ProjectConversationHistory
            activeConversationId="conversation-1"
            isConversationHistoryLoading={false}
            hasRenderableConversationHistory
            activeConversationHistory={activeConversationHistory}
            activeFlowRunRequestsById={new Map()}
            activeFlowLaunchesById={new Map()}
            latestFlowRunRequestId={null}
            latestFlowLaunchId={null}
            expandedToolCalls={expandedToolCalls}
            expandedThinkingEntries={expandedThinkingEntries}
            pendingFlowRunRequestId={null}
            formatConversationTimestamp={formatConversationTimestamp}
            onToggleToolCallExpanded={(toolCallId) =>
                setExpandedToolCalls((current) => ({
                    ...current,
                    [toolCallId]: !current[toolCallId],
                }))
            }
            onToggleThinkingEntryExpanded={(entryId) =>
                setExpandedThinkingEntries((current) => ({
                    ...current,
                    [entryId]: !current[entryId],
                }))
            }
            onReviewFlowRunRequest={vi.fn()}
            onOpenFlowRun={vi.fn()}
        />
    )
}

const makeMessageEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'message' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'message' }> => ({
    id: overrides.id ?? 'message-1',
    kind: 'message',
    role: overrides.role ?? 'assistant',
    content: overrides.content ?? 'Plain text response.',
    timestamp: overrides.timestamp ?? '2026-04-16T15:27:47Z',
    status: overrides.status ?? 'complete',
    error: overrides.error ?? null,
    presentation: overrides.presentation ?? 'default',
})

const makeToolCallEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'tool_call' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'tool_call' }> => ({
    id: overrides.id ?? 'entry-tool-1',
    kind: 'tool_call',
    role: 'system',
    timestamp: overrides.timestamp ?? '2026-04-16T15:27:47Z',
    toolCall: overrides.toolCall ?? {
        id: 'tool-1',
        kind: 'command_execution',
        status: 'completed',
        title: 'List files',
        command: '/bin/zsh -lc printf "**literal**"',
        output: '[README](https://example.com)',
        filePaths: [],
    },
})

describe('ProjectConversationHistory', () => {
    it('renders markdown semantics for normal assistant messages', () => {
        renderHistory([
            makeMessageEntry({
                content: '## Steps\n\nUse **bold** text and *italics* in the response.',
            }),
        ])

        const history = screen.getByTestId('project-ai-conversation-history-list')
        expect(within(history).getByRole('heading', { level: 2, name: 'Steps' })).toBeVisible()
        expect(within(history).getByText('bold', { selector: 'strong' })).toBeVisible()
        expect(within(history).getByText('italics', { selector: 'em' })).toBeVisible()
        expect(within(history).queryByText('**bold**')).not.toBeInTheDocument()
    })

    it('renders assistant fenced code blocks as preformatted code', () => {
        renderHistory([
            makeMessageEntry({
                content: '```bash\nnpm test\n```',
            }),
        ])

        const history = screen.getByTestId('project-ai-conversation-history-list')
        const codeBlock = history.querySelector('pre > code')
        expect(codeBlock).not.toBeNull()
        expect(codeBlock).toHaveTextContent('npm test')
    })

    it('renders assistant markdown links as plain labels without anchors', () => {
        renderHistory([
            makeMessageEntry({
                content: 'Read [the docs](https://example.com/docs) before continuing.',
            }),
        ])

        const history = screen.getByTestId('project-ai-conversation-history-list')
        expect(within(history).getByText('the docs')).toBeVisible()
        expect(within(history).queryByRole('link', { name: 'the docs' })).not.toBeInTheDocument()
        expect(history).not.toHaveTextContent('https://example.com/docs')
    })

    it('keeps assistant markdown image syntax from rendering images', () => {
        renderHistory([
            makeMessageEntry({
                content: 'Diagram: ![Architecture overview](https://example.com/diagram.png)',
            }),
        ])

        const history = screen.getByTestId('project-ai-conversation-history-list')
        expect(within(history).getByText('Architecture overview')).toBeVisible()
        expect(within(history).queryByRole('img', { name: 'Architecture overview' })).not.toBeInTheDocument()
    })

    it('keeps user messages literal even when they contain markdown syntax', () => {
        renderHistory([
            makeMessageEntry({
                role: 'user',
                content: '**literal** [docs](https://example.com/docs)',
            }),
        ])

        const history = screen.getByTestId('project-ai-conversation-history-list')
        expect(history).toHaveTextContent('**literal** [docs](https://example.com/docs)')
        expect(within(history).queryByRole('link', { name: 'docs' })).not.toBeInTheDocument()
    })

    it('keeps thinking summaries collapsed until expanded', async () => {
        const user = userEvent.setup()

        render(
            <InteractiveHistory
                activeConversationHistory={[
                    makeMessageEntry({
                        id: 'segment-reasoning-1',
                        presentation: 'thinking',
                        status: 'streaming',
                        content: '**Considering proposal** Smallest safe change first.',
                    }),
                ]}
            />,
        )

        expect(screen.getByText('Considering proposal')).toBeVisible()
        expect(screen.queryByText('Smallest safe change first.')).not.toBeInTheDocument()

        await user.click(screen.getByTestId('project-thinking-toggle-segment-reasoning-1'))
        expect(screen.getByText('Smallest safe change first.')).toBeVisible()
    })

    it('keeps tool command and output rows literal and unchanged', async () => {
        const user = userEvent.setup()

        render(<InteractiveHistory activeConversationHistory={[makeToolCallEntry()]} />)

        const history = screen.getByTestId('project-ai-conversation-history-list')
        expect(history).toHaveTextContent('/bin/zsh -lc printf "**literal**"')
        expect(screen.queryByText('[README](https://example.com)')).not.toBeInTheDocument()

        await user.click(screen.getByTestId('project-tool-call-toggle-tool-1'))

        expect(screen.getByText('[README](https://example.com)')).toBeVisible()
        expect(within(history).queryByRole('link', { name: 'README' })).not.toBeInTheDocument()
    })
})
