import { useState } from 'react'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { groupTranscript, type TranscriptItemClass } from '@/components/app/transcript/groupTranscript'
import {
    AssistantTurnHeader,
    EventLine,
    RunCard,
    ShowThinkingToggle,
    ThinkingRow,
    ToolCallGroupRow,
    ToolCallRow,
    type TranscriptToolCallEntry,
} from '@/components/app/transcript/SegmentRows'
import { Transcript } from '@/components/app/transcript/Transcript'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { SidePanelRow } from '@/components/app/side-panel-row'
import { StatusPill } from '@/components/app/status'

const loadOutput = vi.hoisted(() => vi.fn(async () => ({ output: 'full output', output_size: 4096 })))
vi.mock('@/features/projects/services/conversationArtifacts', () => ({ loadConversationSegmentToolOutput: loadOutput }))

const entry = (id: string, command: string, status: TranscriptToolCallEntry['toolCall']['status'] = 'completed', output?: string): TranscriptToolCallEntry => ({
    id,
    timestamp: '2026-10-01T10:00:00Z',
    toolCall: { id, kind: 'command_execution', status, title: 'Ran command', command, output: output ?? null, filePaths: [] },
})

const ENTRIES = [
    entry('a', "/bin/zsh -lc 'git status --short'", 'completed', 'M src/App.tsx'),
    entry('b', "/bin/zsh -lc 'npm test'", 'failed', 'exit 1'),
    entry('c', 'cat AGENTS.md'),
]

function Group() {
    const [open, setOpen] = useState(false)
    const [expanded, setExpanded] = useState<Record<string, boolean>>({})
    const [block] = groupTranscript(ENTRIES, (item): TranscriptItemClass => ({ kind: 'tool_call', toolCall: item.toolCall }))
    if (block.kind !== 'tool_group') throw new Error('expected a group')
    return (
        <ul>
            <ToolCallGroupRow summary={block.summary} isExpanded={open} onToggle={() => setOpen(!open)} testId="group">
                {block.members.map((member) => (
                    <ToolCallRow
                        key={member.toolCall.id}
                        variant="line"
                        entry={member.item}
                        isExpanded={expanded[member.toolCall.id] === true}
                        onToggleToolCallExpanded={(id) => setExpanded((current) => ({ ...current, [id]: !current[id] }))}
                        testIdPrefix="t"
                    />
                ))}
            </ToolCallGroupRow>
        </ul>
    )
}

describe('ToolCallGroupRow', () => {
    it('summarizes the group, counts failures, and expands to compact command lines', () => {
        render(<Group />)
        const toggle = screen.getByTestId('group')
        expect(toggle).toHaveTextContent('Ran 3 commands· 1 failed')
        expect(toggle).toHaveAttribute('aria-expanded', 'false')
        expect(screen.queryByText('git status --short')).not.toBeInTheDocument()

        fireEvent.click(toggle)
        expect(toggle).toHaveAttribute('aria-expanded', 'true')
        // Lines show the command without its shell wrapper and keep the original in the tooltip.
        const first = screen.getByTestId('t-tool-call-toggle-a')
        expect(first).toHaveTextContent('git status --short')
        expect(first).not.toHaveTextContent('/bin/zsh')
        expect(first).toHaveAttribute('title', "/bin/zsh -lc 'git status --short'")
        expect(screen.getByTestId('t-tool-call-toggle-b')).toHaveTextContent('Failed')
        expect(screen.getByTestId('t-tool-call-toggle-a')).not.toHaveTextContent('Failed')

        // A line still reveals that call's output, with the full command.
        fireEvent.click(first)
        expect(first).toHaveAttribute('aria-expanded', 'true')
        expect(screen.getByText('M src/App.tsx', { selector: 'pre' })).toBeVisible()
        expect(screen.getByText("/bin/zsh -lc 'git status --short'", { selector: 'p' })).toBeVisible()
    })

    it('says Running… while a call is still running', () => {
        const [block] = groupTranscript([entry('r', 'sleep 1', 'running')], (item): TranscriptItemClass => ({ kind: 'tool_call', toolCall: item.toolCall }))
        if (block.kind !== 'tool_group') throw new Error('expected a group')
        render(<ul><ToolCallGroupRow summary={block.summary} isExpanded={false} onToggle={() => {}} testId="g" /></ul>)
        expect(screen.getByTestId('g')).toHaveTextContent('Ran 1 command· Running…')
    })
})

describe('transcript rows', () => {
    it('renders a thinking row without a Thought: prefix', () => {
        render(<ul><ThinkingRow entry={{ id: 'th', role: 'assistant', status: 'complete', timestamp: '', content: '**Checking tests**\n\nbody', presentation: 'thinking' }} isExpanded={false} onToggleThinkingEntryExpanded={() => {}} formatConversationTimestamp={() => ''} /></ul>)
        expect(screen.getByRole('button', { name: 'Checking tests' })).toBeVisible()
        expect(screen.queryByText(/Thought:/)).not.toBeInTheDocument()
    })

    it('toggles Show thinking through aria-pressed', () => {
        const onPressedChange = vi.fn()
        const { rerender } = render(<ShowThinkingToggle pressed={false} onPressedChange={onPressedChange} />)
        const button = screen.getByRole('button', { name: 'Show thinking' })
        expect(button).toHaveAttribute('aria-pressed', 'false')
        fireEvent.click(button)
        expect(onPressedChange).toHaveBeenCalledWith(true)
        rerender(<ShowThinkingToggle pressed onPressedChange={onPressedChange} />)
        expect(button).toHaveAttribute('aria-pressed', 'true')
    })

    it('opens an assistant turn with the mark, Spark and muted meta', () => {
        render(<AssistantTurnHeader meta="Oct 1 · worked 16s" data-testid="turn" />)
        const header = screen.getByTestId('turn')
        expect(header).toHaveTextContent('Spark')
        expect(header).toHaveTextContent('worked 16s')
        expect(within(header).getByRole('presentation', { hidden: true })).toHaveAttribute('src', '/assets/spark-app-icon.png')
    })

    it('renders event lines and run cards with the subject, status and time', () => {
        render(
            <ul>
                <li><EventLine at="Oct 1" data-testid="event"><a href="#run">Implement change</a> ended · Completed</EventLine></li>
                <li><RunCard data-testid="card" kicker="Launched run · Implement Change" title={<a href="#run">Implement and validate</a>} tone="success" status="Completed" duration="39m" /></li>
            </ul>,
        )
        expect(screen.getByTestId('event')).toHaveTextContent('Implement change ended · CompletedOct 1')
        const card = screen.getByTestId('card')
        expect(within(card).getByRole('link', { name: 'Implement and validate' })).toHaveAttribute('href', '#run')
        expect(card).toHaveTextContent('Launched run · Implement Change')
        expect(card).toHaveTextContent('Completed · 39m')
        expect(card.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'success')
    })
})

describe('shared primitives', () => {
    it('marks a selected side-panel row as current and draws a hollow dot on request', () => {
        const { rerender } = render(<SidePanelRow tone="success" title="Implement CR-0143" meta="Implement Change · 39m" trailing="2" />)
        const row = screen.getByRole('button', { name: /Implement CR-0143/ })
        expect(row).not.toHaveAttribute('aria-current')
        expect(row).toHaveTextContent('Implement Change · 39m')
        expect(row).toHaveTextContent('2')
        rerender(<SidePanelRow tone="neutral" hollow selected title="Draft mission" />)
        expect(screen.getByRole('button', { name: 'Draft mission' })).toHaveAttribute('aria-current', 'true')
        expect(document.querySelector('[data-slot="status-dot"]')?.className).toContain('border')
    })

    it('renders a status pill with its tone', () => {
        render(<StatusPill tone="destructive">Failed</StatusPill>)
        const pill = screen.getByText('Failed')
        expect(pill).toHaveAttribute('data-tone', 'destructive')
        expect(pill.className).toContain('rounded-full')
        expect(pill.className).not.toContain('uppercase')
    })
})

describe('Transcript', () => {
    const at = '2026-10-01T10:00:00Z'
    const call = (id: string, extra: Partial<TranscriptToolCallEntry['toolCall']> = {}): ConversationTimelineEntry => ({
        id: `s-${id}`, kind: 'tool_call', role: 'system', timestamp: at,
        toolCall: { id, kind: 'command_execution', status: 'completed', title: 'Ran command', command: `run ${id}`, output: `${id} output`, filePaths: [], ...extra },
    })
    const message = (id: string, role: 'user' | 'assistant', content: string): ConversationTimelineEntry => ({ id, kind: 'message', role, content, timestamp: at, status: 'complete' })
    const thought = (id: string, status: 'streaming' | 'complete'): ConversationTimelineEntry => ({
        id, kind: 'message', role: 'assistant', content: `**Thought ${id}**\n\ndetails`, timestamp: at, status, presentation: 'thinking',
    })
    const view = (entries: ConversationTimelineEntry[], props: Partial<Parameters<typeof Transcript>[0]> = {}) => (
        <Transcript testIdPrefix="t" entries={entries} showThinking={false} formatTimestamp={() => ''} {...props} />
    )

    it('folds consecutive tool calls into one group whose lines expand, and keeps a lone call a plain row', () => {
        render(view([call('a'), call('b'), message('m', 'assistant', 'Between.'), call('c')]))
        const group = screen.getByTestId('t-tool-group-a')
        expect(group).toHaveTextContent('Ran 2 commands')
        expect(screen.queryByTestId('t-tool-call-toggle-a')).toBeNull()
        fireEvent.click(group)
        fireEvent.click(screen.getByTestId('t-tool-call-toggle-a'))
        expect(screen.getByText('a output')).toBeVisible()
        expect(screen.queryByTestId('t-tool-group-c')).toBeNull()
        expect(screen.getByTestId('t-tool-call-toggle-c')).toHaveTextContent('run c')
    })

    it('marks a hidden live thought, hides a finished one, and shows both on request', () => {
        const entries = [thought('live', 'streaming'), message('m', 'assistant', 'Answer.'), thought('done', 'complete')]
        const { rerender } = render(view(entries))
        expect(screen.getAllByText('Thinking')).toHaveLength(1)
        expect(screen.queryByText('Thought live')).toBeNull()
        expect(screen.queryByText('Thought done')).toBeNull()
        rerender(view(entries, { showThinking: true }))
        expect(screen.getByText('Thought live')).toBeVisible()
        expect(screen.getByText('Thought done')).toBeVisible()
        expect(screen.queryByText('Thinking')).toBeNull()
        // Readable without a click; each entry still collapses and reopens on its own.
        expect(screen.getAllByText('details')).toHaveLength(2)
        fireEvent.click(screen.getByTestId('t-thinking-toggle-done'))
        expect(screen.getAllByText('details')).toHaveLength(1)
        fireEvent.click(screen.getByTestId('t-thinking-toggle-done'))
        expect(screen.getAllByText('details')).toHaveLength(2)
    })

    it('keeps thoughts between tool calls visible when shown, and folds across them when hidden', () => {
        const entries = [call('a'), thought('between', 'complete'), call('b'), call('c')]
        const { rerender } = render(view(entries, { showThinking: true }))
        expect(screen.getByText('Thought between')).toBeVisible()
        expect(screen.getByTestId('t-tool-call-toggle-a')).toBeVisible()
        expect(screen.getByTestId('t-tool-group-b')).toHaveTextContent('Ran 2 commands')
        rerender(view(entries))
        expect(screen.queryByText('Thought between')).toBeNull()
        expect(screen.getByTestId('t-tool-group-a')).toHaveTextContent('Ran 3 commands')
    })

    it('fetches a truncated call\'s full output once when it opens within a conversation, and never without one', async () => {
        const truncated = call('big', { output: 'preview', outputSize: 4096, outputTruncated: true })
        loadOutput.mockClear()
        const { unmount } = render(view([truncated]))
        fireEvent.click(screen.getByTestId('t-tool-call-toggle-big'))
        expect(screen.getByText('preview')).toBeVisible()
        expect(loadOutput).not.toHaveBeenCalled()
        unmount()

        render(view([truncated], { conversation: { id: 'conv', projectPath: '/project' } }))
        const toggle = screen.getByTestId('t-tool-call-toggle-big')
        fireEvent.click(toggle)
        expect(await screen.findByText('full output')).toBeVisible()
        fireEvent.click(toggle)
        fireEvent.click(toggle)
        await waitFor(() => expect(screen.getByText('full output')).toBeVisible())
        expect(loadOutput).toHaveBeenCalledTimes(1)
        expect(loadOutput).toHaveBeenCalledWith('conv', 's-big', '/project')
    })

    it('opens each assistant turn with a header carrying the worked meta, or renders one flat list', () => {
        const entries: ConversationTimelineEntry[] = [
            message('u1', 'user', 'Go.'),
            call('a'),
            { id: 'sep', kind: 'final_separator', role: 'system', timestamp: at, label: 'Worked for 16s' },
            message('a1', 'assistant', 'Done.'),
            message('u2', 'user', 'Again.'),
            message('a2', 'assistant', 'Done again.'),
        ]
        const { rerender } = render(view(entries))
        expect(screen.getAllByText('Spark', { selector: 'span' })).toHaveLength(2)
        expect(screen.getByTestId('t-assistant-turn-s-a')).toHaveTextContent(/worked 16s/)
        expect(screen.queryByText('Worked for 16s')).toBeNull()
        rerender(view(entries.filter((entry) => entry.kind !== 'final_separator'), { turns: false }))
        expect(screen.queryByText('Spark', { selector: 'span' })).toBeNull()
        expect(screen.getByText('Done again.')).toBeVisible()
    })

    it('lets a surface hide an entry with null and fall back to the default row with undefined', () => {
        render(view([message('u1', 'user', 'Hidden.'), message('a1', 'assistant', 'Default.'), message('a2', 'assistant', 'Replaced.')], {
            turns: false,
            renderEntry: (item) => item.id === 'u1' ? null : item.id === 'a2' ? <li data-testid="custom">Custom</li> : undefined,
        }))
        expect(screen.queryByText('Hidden.')).toBeNull()
        expect(screen.getByText('Default.')).toBeVisible()
        expect(screen.getByTestId('custom')).toHaveTextContent('Custom')
        expect(screen.queryByText('Replaced.')).toBeNull()
    })
})
