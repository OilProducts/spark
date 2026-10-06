import { useState } from 'react'
import { fireEvent, render, screen, within } from '@testing-library/react'
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
import { SidePanelRow } from '@/components/app/side-panel-row'
import { StatusPill } from '@/components/app/status'

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
                {block.members.map((member) => member.kind === 'tool_call' ? (
                    <ToolCallRow
                        key={member.toolCall.id}
                        variant="line"
                        entry={member.item}
                        isExpanded={expanded[member.toolCall.id] === true}
                        onToggleToolCallExpanded={(id) => setExpanded((current) => ({ ...current, [id]: !current[id] }))}
                        testIdPrefix="t"
                    />
                ) : null)}
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
