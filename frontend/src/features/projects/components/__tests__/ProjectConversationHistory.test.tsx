import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ProjectConversationHistory } from '@/features/projects/components/ProjectConversationHistory'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
    ProjectProposedPlan,
} from '@/features/projects/model/types'

const markdownRenderSpy = vi.hoisted(() => vi.fn())
const flowRunRequestEntryRenderSpy = vi.hoisted(() => vi.fn())

vi.mock('@/features/projects/components/ProjectConversationMarkdown', async () => {
    const actual = await vi.importActual<typeof import('@/features/projects/components/ProjectConversationMarkdown')>(
        '@/features/projects/components/ProjectConversationMarkdown',
    )
    const React = await import('react')
    return {
        ProjectConversationMarkdown: (props: { content: string }) => {
            markdownRenderSpy(props.content)
            return React.createElement(actual.ProjectConversationMarkdown, props)
        },
    }
})

vi.mock('@/features/projects/components/ProjectArtifactEntries', async () => {
    const actual = await vi.importActual<typeof import('@/features/projects/components/ProjectArtifactEntries')>(
        '@/features/projects/components/ProjectArtifactEntries',
    )
    const React = await import('react')
    return {
        ProjectFlowRunRequestEntry: (props: Parameters<typeof actual.ProjectFlowRunRequestEntry>[0]) => {
            flowRunRequestEntryRenderSpy(props.flowRunRequest?.id ?? null)
            return React.createElement(actual.ProjectFlowRunRequestEntry, props)
        },
        ProjectFlowLaunchEntry: actual.ProjectFlowLaunchEntry,
    }
})

const formatConversationTimestamp = (value: string) => `formatted:${value}`
type FlowRunRequestTimelineEntry = Extract<
    ConversationTimelineEntry,
    { kind: 'flow_run_request' | 'flow_launch' }
> & { kind: 'flow_run_request' }

const makeHistoryElement = (
    activeConversationHistory: ConversationTimelineEntry[],
    overrides: {
        activeFlowRunRequestsById?: Map<string, ProjectFlowRunRequest>
        activeFlowLaunchesById?: Map<string, ProjectFlowLaunch>
        activeProposedPlansById?: Map<string, ProjectProposedPlan>
        latestFlowRunRequestId?: string | null
        onOpenFlowRun?: (request: { run_id?: string | null; flow_name: string }) => void
        onReviewFlowRunRequest?: (flowRunRequest: ProjectFlowRunRequest, disposition: 'approved' | 'rejected') => void | Promise<void>
        onSubmitRequestUserInput?: (requestId: string, answers: Record<string, string>) => void | Promise<void>
        onReviewProposedPlan?: (proposedPlan: ProjectProposedPlan, disposition: 'approved' | 'rejected', reviewNote?: string | null) => void | Promise<void>
        pendingFlowRunRequestId?: string | null
        pendingProposedPlanId?: string | null
        requestUserInputActionError?: string | null
        submittingRequestUserInputIds?: Record<string, boolean>
    } = {},
) =>
    <ProjectConversationHistory
        activeConversationId="conversation-1"
        activeProjectPath="/tmp/project"
        isConversationHistoryLoading={false}
        hasRenderableConversationHistory={activeConversationHistory.length > 0}
        activeConversationHistory={activeConversationHistory}
        activeFlowRunRequestsById={overrides.activeFlowRunRequestsById ?? new Map()}
        activeFlowLaunchesById={overrides.activeFlowLaunchesById ?? new Map()}
        activeProposedPlansById={overrides.activeProposedPlansById ?? new Map()}
        latestFlowRunRequestId={overrides.latestFlowRunRequestId ?? null}
        latestFlowLaunchId={null}
        expandedToolCalls={{}}
        expandedThinkingEntries={{}}
        pendingFlowRunRequestId={overrides.pendingFlowRunRequestId ?? null}
        pendingProposedPlanId={overrides.pendingProposedPlanId ?? null}
        requestUserInputActionError={overrides.requestUserInputActionError ?? null}
        submittingRequestUserInputIds={overrides.submittingRequestUserInputIds ?? {}}
        formatConversationTimestamp={formatConversationTimestamp}
        onSubmitRequestUserInput={overrides.onSubmitRequestUserInput ?? vi.fn()}
        onToggleToolCallExpanded={vi.fn()}
        onToggleThinkingEntryExpanded={vi.fn()}
        onReviewFlowRunRequest={overrides.onReviewFlowRunRequest ?? vi.fn()}
        onReviewProposedPlan={overrides.onReviewProposedPlan ?? vi.fn()}
        onOpenFlowRun={overrides.onOpenFlowRun ?? vi.fn()}
    />

const renderHistory = (
    activeConversationHistory: ConversationTimelineEntry[],
    overrides: Parameters<typeof makeHistoryElement>[1] = {},
) => render(makeHistoryElement(activeConversationHistory, overrides))

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
            activeProjectPath="/tmp/project"
            isConversationHistoryLoading={false}
            hasRenderableConversationHistory
            activeConversationHistory={activeConversationHistory}
            activeFlowRunRequestsById={new Map()}
            activeFlowLaunchesById={new Map()}
            activeProposedPlansById={new Map()}
            latestFlowRunRequestId={null}
            latestFlowLaunchId={null}
            expandedToolCalls={expandedToolCalls}
            expandedThinkingEntries={expandedThinkingEntries}
            pendingFlowRunRequestId={null}
            pendingProposedPlanId={null}
            requestUserInputActionError={null}
            submittingRequestUserInputIds={{}}
            formatConversationTimestamp={formatConversationTimestamp}
            onSubmitRequestUserInput={vi.fn()}
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
            onReviewProposedPlan={vi.fn()}
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

const makeModeChangeEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'mode_change' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'mode_change' }> => ({
    id: overrides.id ?? 'mode-change-1',
    kind: 'mode_change',
    role: 'system',
    timestamp: overrides.timestamp ?? '2026-04-16T15:27:47Z',
    mode: overrides.mode ?? 'plan',
})

const makeContextCompactionEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'context_compaction' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'context_compaction' }> => ({
    id: overrides.id ?? 'context-compaction-1',
    kind: 'context_compaction',
    role: 'system',
    timestamp: overrides.timestamp ?? '2026-04-16T15:27:47Z',
    content: overrides.content ?? 'Context compacted to continue the turn.',
    status: overrides.status ?? 'complete',
})

const makeRequestUserInputEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'request_user_input' }> => ({
    id: overrides.id ?? 'request-user-input-1',
    kind: 'request_user_input',
    role: 'system',
    timestamp: overrides.timestamp ?? '2026-04-17T12:00:00Z',
    content: overrides.content ?? 'Which path should I take?',
    status: overrides.status ?? 'pending',
    requestUserInput: overrides.requestUserInput ?? {
        requestId: 'request-1',
        status: 'pending',
        questions: [
            {
                id: 'path_choice',
                header: 'Path',
                question: 'Which path should I take?',
                questionType: 'MULTIPLE_CHOICE',
                options: [
                    {
                        label: 'Inline card',
                        description: 'Keep the request inside the timeline.',
                    },
                    {
                        label: 'Composer takeover',
                        description: 'Move the unanswered state into the composer.',
                    },
                ],
                allowOther: true,
                isSecret: false,
            },
            {
                id: 'constraints',
                header: 'Constraints',
                question: 'What constraints matter?',
                questionType: 'FREEFORM',
                options: [],
                allowOther: false,
                isSecret: false,
            },
        ],
        answers: {},
        submittedAt: null,
    },
})

const makePlanEntry = (
    overrides: Partial<Extract<ConversationTimelineEntry, { kind: 'plan' }>> = {},
): Extract<ConversationTimelineEntry, { kind: 'plan' }> => ({
    id: overrides.id ?? 'plan-1',
    kind: 'plan',
    role: 'assistant',
    content: overrides.content ?? '1. Ship the regression test.\n2. Validate the real session path.',
    timestamp: overrides.timestamp ?? '2026-04-16T15:27:47Z',
    status: overrides.status ?? 'complete',
    artifactId: overrides.artifactId ?? null,
    error: overrides.error ?? null,
})

const makeFlowRunRequestEntry = (
    overrides: Partial<FlowRunRequestTimelineEntry> = {},
): FlowRunRequestTimelineEntry => ({
    id: overrides.id ?? 'flow-run-request-entry-1',
    kind: 'flow_run_request',
    role: 'system',
    artifactId: overrides.artifactId ?? 'flow-run-request-1',
    timestamp: overrides.timestamp ?? '2026-04-16T15:28:00Z',
})

const makeFlowRunRequest = (
    overrides: Partial<ProjectFlowRunRequest> = {},
): ProjectFlowRunRequest => ({
    id: overrides.id ?? 'flow-run-request-1',
    created_at: overrides.created_at ?? '2026-04-16T15:27:47Z',
    updated_at: overrides.updated_at ?? '2026-04-16T15:28:00Z',
    flow_name: overrides.flow_name ?? 'software-development/implement-change-request.dot',
    summary: overrides.summary ?? 'Implement approved change request',
    project_path: overrides.project_path ?? '/tmp/project',
    conversation_id: overrides.conversation_id ?? 'conversation-1',
    source_turn_id: overrides.source_turn_id ?? 'turn-assistant-1',
    status: overrides.status ?? 'pending',
    source_segment_id: overrides.source_segment_id ?? 'segment-flow-run-request-1',
    goal: overrides.goal ?? null,
    launch_context: overrides.launch_context ?? null,
    model: overrides.model ?? null,
    llm_provider: overrides.llm_provider ?? null,
    reasoning_effort: overrides.reasoning_effort ?? null,
    run_id: overrides.run_id ?? null,
    launch_error: overrides.launch_error ?? null,
    review_message: overrides.review_message ?? null,
})

const makeProposedPlan = (
    overrides: Partial<ProjectProposedPlan> = {},
): ProjectProposedPlan => ({
    id: overrides.id ?? 'proposed-plan-1',
    created_at: overrides.created_at ?? '2026-04-16T15:27:47Z',
    updated_at: overrides.updated_at ?? '2026-04-16T15:28:00Z',
    title: overrides.title ?? 'Ship the regression',
    content: overrides.content ?? '1. Ship the regression test.\n2. Validate the real session path.',
    project_path: overrides.project_path ?? '/tmp/project',
    conversation_id: overrides.conversation_id ?? 'conversation-1',
    source_turn_id: overrides.source_turn_id ?? 'turn-assistant-1',
    source_segment_id: overrides.source_segment_id ?? 'plan-1',
    status: overrides.status ?? 'pending_review',
    review_note: overrides.review_note ?? null,
    written_change_request_path: overrides.written_change_request_path ?? null,
    flow_launch_id: overrides.flow_launch_id ?? null,
    run_id: overrides.run_id ?? null,
    launch_error: overrides.launch_error ?? null,
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

    it('does not rerender an older unchanged assistant markdown row when the latest assistant markdown changes', () => {
        const olderAssistantMessage = makeMessageEntry({
            id: 'assistant-old',
            content: '## Older answer\n\nKeep **this** parsed markdown stable.',
            timestamp: '2026-04-16T15:27:47Z',
        })
        const latestAssistantMessage = makeMessageEntry({
            id: 'assistant-latest',
            content: 'Streaming **first** chunk.',
            timestamp: '2026-04-16T15:28:47Z',
            status: 'streaming',
        })

        const { rerender } = renderHistory([
            olderAssistantMessage,
            latestAssistantMessage,
        ])
        markdownRenderSpy.mockClear()

        rerender(makeHistoryElement([
            olderAssistantMessage,
            {
                ...latestAssistantMessage,
                content: 'Streaming **second** chunk.',
            },
        ]))

        expect(markdownRenderSpy).toHaveBeenCalledTimes(1)
        expect(markdownRenderSpy).toHaveBeenCalledWith('Streaming **second** chunk.')
    })

    it('does not rerender unchanged plan or artifact rows when sibling streaming markdown changes', () => {
        const planEntry = makePlanEntry({
            id: 'plan-stable',
            artifactId: 'proposed-plan-1',
            content: '## Stable plan\n\nKeep **this plan** parsed once.',
            timestamp: '2026-04-16T15:27:47Z',
        })
        const flowRunRequestEntry = makeFlowRunRequestEntry({
            id: 'flow-run-request-entry-stable',
            artifactId: 'flow-run-request-1',
            timestamp: '2026-04-16T15:28:00Z',
        })
        const latestAssistantMessage = makeMessageEntry({
            id: 'assistant-latest',
            content: 'Streaming **first** chunk.',
            timestamp: '2026-04-16T15:28:47Z',
            status: 'streaming',
        })
        const activeFlowRunRequestsById = new Map([
            ['flow-run-request-1', makeFlowRunRequest()],
        ])
        const activeProposedPlansById = new Map([
            ['proposed-plan-1', makeProposedPlan()],
        ])
        const stableOverrides = {
            activeFlowRunRequestsById,
            activeProposedPlansById,
            latestFlowRunRequestId: 'flow-run-request-1',
            onOpenFlowRun: vi.fn(),
            onReviewFlowRunRequest: vi.fn(),
            onReviewProposedPlan: vi.fn(),
        }

        const { rerender } = renderHistory([
            planEntry,
            flowRunRequestEntry,
            latestAssistantMessage,
        ], stableOverrides)
        markdownRenderSpy.mockClear()
        flowRunRequestEntryRenderSpy.mockClear()

        rerender(makeHistoryElement([
            planEntry,
            flowRunRequestEntry,
            {
                ...latestAssistantMessage,
                content: 'Streaming **second** chunk.',
            },
        ], stableOverrides))

        expect(markdownRenderSpy).toHaveBeenCalledTimes(1)
        expect(markdownRenderSpy).toHaveBeenCalledWith('Streaming **second** chunk.')
        expect(markdownRenderSpy).not.toHaveBeenCalledWith('## Stable plan\n\nKeep **this plan** parsed once.')
        expect(flowRunRequestEntryRenderSpy).not.toHaveBeenCalled()
    })

    it('keeps rendering markdown semantics for the latest streaming assistant row as it updates', () => {
        const latestAssistantMessage = makeMessageEntry({
            id: 'assistant-streaming',
            content: '## Draft\n\nUse **markdown** now.',
            timestamp: '2026-04-16T15:28:47Z',
            status: 'streaming',
        })

        const { rerender } = renderHistory([latestAssistantMessage])

        expect(screen.getByRole('heading', { level: 2, name: 'Draft' })).toBeVisible()
        expect(screen.getByText('markdown', { selector: 'strong' })).toBeVisible()

        rerender(makeHistoryElement([
            {
                ...latestAssistantMessage,
                content: '## Draft\n\nUse **updated markdown** now.',
            },
        ]))

        expect(screen.getByRole('heading', { level: 2, name: 'Draft' })).toBeVisible()
        expect(screen.getByText('updated markdown', { selector: 'strong' })).toBeVisible()
        expect(screen.queryByText('**updated markdown**')).not.toBeInTheDocument()
    })

    it('renders mode-change rows as centered inline system markers', () => {
        renderHistory([
            makeModeChangeEntry(),
        ])

        const row = screen.getByTestId('project-mode-change-row-mode-change-1')
        expect(within(row).getByText('Switched to Plan mode')).toBeVisible()
    })

    it('renders context compaction rows as centered inline system markers', () => {
        renderHistory([
            makeContextCompactionEntry(),
        ])

        const row = screen.getByTestId('project-context-compaction-row-context-compaction-1')
        expect(within(row).getByText('Context compacted to continue the turn.')).toBeVisible()
    })

    it('renders plan entries as dedicated markdown cards', () => {
        renderHistory([
            makePlanEntry({
                content: '## Proposed steps\n\n1. Add the transport regression.\n2. Run validation.',
            }),
        ])

        const planCard = screen.getByTestId('project-plan-card-plan-1')
        expect(within(planCard).getByText('Proposed Plan')).toBeVisible()
        expect(within(planCard).getByRole('heading', { level: 2, name: 'Proposed steps' })).toBeVisible()
        expect(within(planCard).getByText('Add the transport regression.')).toBeVisible()
    })

    it('renders pending proposed plan review controls and submits the inline note', async () => {
        const user = userEvent.setup()
        const onReviewProposedPlan = vi.fn()

        renderHistory([
            makePlanEntry({
                artifactId: 'proposed-plan-1',
            }),
        ], {
            activeProposedPlansById: new Map([
                ['proposed-plan-1', {
                    id: 'proposed-plan-1',
                    created_at: '2026-04-16T15:27:47Z',
                    updated_at: '2026-04-16T15:27:47Z',
                    title: 'Ship the regression',
                    content: '1. Ship the regression test.\n2. Validate the real session path.',
                    project_path: '/tmp/project',
                    conversation_id: 'conversation-1',
                    source_turn_id: 'turn-assistant-1',
                    source_segment_id: 'plan-1',
                    status: 'pending_review',
                    review_note: null,
                    written_change_request_path: null,
                    flow_launch_id: null,
                    run_id: null,
                    launch_error: null,
                }],
            ]),
            onReviewProposedPlan,
        })

        expect(screen.getByText('Pending review')).toBeVisible()
        await user.type(screen.getByTestId('project-proposed-plan-review-note-proposed-plan-1'), 'Ship it.')
        await user.click(screen.getByTestId('project-proposed-plan-approve-button-proposed-plan-1'))

        expect(onReviewProposedPlan).toHaveBeenCalledWith(
            expect.objectContaining({ id: 'proposed-plan-1' }),
            'approved',
            'Ship it.',
        )
    })

    it('renders approved proposed plans with written path and run metadata', () => {
        renderHistory([
            makePlanEntry({
                artifactId: 'proposed-plan-1',
            }),
        ], {
            activeFlowLaunchesById: new Map([
                ['flow-launch-1', {
                    id: 'flow-launch-1',
                    created_at: '2026-04-16T15:27:47Z',
                    updated_at: '2026-04-16T15:28:10Z',
                    flow_name: 'software-development/implement-change-request.dot',
                    summary: 'Implement approved change request',
                    project_path: '/tmp/project',
                    conversation_id: 'conversation-1',
                    source_turn_id: 'turn-assistant-1',
                    source_segment_id: 'segment-artifact-flow-launch-1',
                    status: 'launched',
                    goal: null,
                    launch_context: null,
                    model: null,
                    run_id: 'run-123',
                    launch_error: null,
                }],
            ]),
            activeProposedPlansById: new Map([
                ['proposed-plan-1', {
                    id: 'proposed-plan-1',
                    created_at: '2026-04-16T15:27:47Z',
                    updated_at: '2026-04-16T15:28:10Z',
                    title: 'Ship the regression',
                    content: '1. Ship the regression test.\n2. Validate the real session path.',
                    project_path: '/tmp/project',
                    conversation_id: 'conversation-1',
                    source_turn_id: 'turn-assistant-1',
                    source_segment_id: 'plan-1',
                    status: 'approved',
                    review_note: 'Approved for implementation.',
                    written_change_request_path: '/tmp/project/changes/CR-2026-0007-ship-the-regression/request.md',
                    flow_launch_id: 'flow-launch-1',
                    run_id: 'run-123',
                    launch_error: null,
                }],
            ]),
        })

        const planCard = screen.getByTestId('project-plan-card-plan-1')
        expect(within(planCard).getByText('Approved')).toBeVisible()
        expect(within(planCard).getByText('/tmp/project/changes/CR-2026-0007-ship-the-regression/request.md')).toBeVisible()
        expect(within(planCard).getByText('run-123')).toBeVisible()
        expect(within(planCard).queryByTestId('project-proposed-plan-approve-button-proposed-plan-1')).not.toBeInTheDocument()
    })

    it('renders rejected proposed plans as locked audit history with the review note', () => {
        renderHistory([
            makePlanEntry({
                artifactId: 'proposed-plan-1',
            }),
        ], {
            activeProposedPlansById: new Map([
                ['proposed-plan-1', {
                    id: 'proposed-plan-1',
                    created_at: '2026-04-16T15:27:47Z',
                    updated_at: '2026-04-16T15:28:10Z',
                    title: 'Ship the regression',
                    content: '1. Ship the regression test.\n2. Validate the real session path.',
                    project_path: '/tmp/project',
                    conversation_id: 'conversation-1',
                    source_turn_id: 'turn-assistant-1',
                    source_segment_id: 'plan-1',
                    status: 'rejected',
                    review_note: 'Needs a rollback plan first.',
                    written_change_request_path: null,
                    flow_launch_id: null,
                    run_id: null,
                    launch_error: null,
                }],
            ]),
        })

        const planCard = screen.getByTestId('project-plan-card-plan-1')
        expect(within(planCard).getByText('Rejected')).toBeVisible()
        expect(within(planCard).getByText('Needs a rollback plan first.')).toBeVisible()
        expect(within(planCard).queryByTestId('project-proposed-plan-reject-button-proposed-plan-1')).not.toBeInTheDocument()
    })

    it('renders unanswered request_user_input segments inline and submits answers through the real callback', async () => {
        const user = userEvent.setup()
        const onSubmitRequestUserInput = vi.fn()

        renderHistory([
            makeRequestUserInputEntry(),
        ], {
            onSubmitRequestUserInput,
        })

        const card = screen.getByTestId('project-request-user-input-card-request-user-input-1')
        expect(within(card).getByText('Needs Input')).toBeVisible()
        expect(within(card).getByText('Which path should I take?')).toBeVisible()
        expect(within(card).getByText('What constraints matter?')).toBeVisible()
        expect(screen.queryByTestId('project-pending-questions-preview-note')).not.toBeInTheDocument()

        await user.click(screen.getByTestId('project-request-user-input-option-path_choice-Inline card'))
        await user.type(screen.getByTestId('project-request-user-input-field-constraints'), 'Preserve the inline timeline.')
        await user.click(screen.getByTestId('project-request-user-input-submit-request-1'))

        expect(onSubmitRequestUserInput).toHaveBeenCalledWith('request-1', {
            path_choice: 'Inline card',
            constraints: 'Preserve the inline timeline.',
        })
    })

    it('renders answered request_user_input segments as compact summaries', () => {
        renderHistory([
            makeRequestUserInputEntry({
                status: 'complete',
                requestUserInput: {
                    requestId: 'request-1',
                    status: 'answered',
                    questions: [
                        {
                            id: 'path_choice',
                            header: 'Path',
                            question: 'Which path should I take?',
                            questionType: 'MULTIPLE_CHOICE',
                            options: [],
                            allowOther: false,
                            isSecret: false,
                        },
                    ],
                    answers: {
                        path_choice: 'Inline card',
                    },
                    submittedAt: '2026-04-17T12:01:00Z',
                },
            }),
        ])

        const summary = screen.getByTestId('project-request-user-input-summary-request-user-input-1')
        expect(within(summary).getByText('Answered Request')).toBeVisible()
        expect(within(summary).getByText('Which path should I take?')).toBeVisible()
        expect(within(summary).getByText('Inline card')).toBeVisible()
    })

    it('renders expired request_user_input segments as non-interactive expired cards', () => {
        renderHistory([
            makeRequestUserInputEntry({
                status: 'failed',
                requestUserInput: {
                    requestId: 'request-1',
                    status: 'expired',
                    questions: [
                        {
                            id: 'path_choice',
                            header: 'Path',
                            question: 'Which path should I take?',
                            questionType: 'MULTIPLE_CHOICE',
                            options: [],
                            allowOther: false,
                            isSecret: false,
                        },
                    ],
                    answers: {
                        path_choice: 'Inline card',
                    },
                    submittedAt: '2026-04-17T12:01:00Z',
                },
            }),
        ])

        const expiredCard = screen.getByTestId('project-request-user-input-expired-request-user-input-1')
        expect(within(expiredCard).getByText('Expired Request')).toBeVisible()
        expect(within(expiredCard).getByText('The request expired before this answer could be used. Send a new message to continue.')).toBeVisible()
        expect(within(expiredCard).getByText('Inline card')).toBeVisible()
        expect(within(expiredCard).queryByRole('button', { name: 'Submit' })).not.toBeInTheDocument()
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

    it('loads full output when a truncated tool call is expanded', async () => {
        const user = userEvent.setup()
        const fetchSpy = vi.fn(async () => new Response(
            JSON.stringify({
                output: 'preview plus full output',
                output_size: 24,
            }),
            {
                status: 200,
                headers: { 'Content-Type': 'application/json' },
            },
        ))
        vi.stubGlobal('fetch', fetchSpy)

        render(
            <InteractiveHistory
                activeConversationHistory={[
                    makeToolCallEntry({
                        id: 'segment-tool-1',
                        toolCall: {
                            id: 'tool-1',
                            kind: 'command_execution',
                            status: 'completed',
                            title: 'Large output',
                            command: 'run-large-output',
                            output: 'preview',
                            outputSize: 24,
                            outputTruncated: true,
                            filePaths: [],
                        },
                    }),
                ]}
            />,
        )

        await user.click(screen.getByTestId('project-tool-call-toggle-tool-1'))

        await waitFor(() => {
            expect(screen.getByText('preview plus full output')).toBeVisible()
        })
        expect(fetchSpy).toHaveBeenCalledWith(
            expect.stringContaining('/workspace/api/conversations/conversation-1/segments/segment-tool-1/tool-output?project_path='),
            undefined,
        )
    })
})
