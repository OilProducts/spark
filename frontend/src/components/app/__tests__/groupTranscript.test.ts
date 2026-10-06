import { describe, expect, it } from 'vitest'

import { groupTranscript, summarizeToolGroup, type TranscriptItemClass } from '@/components/app/transcript/groupTranscript'
import type { TranscriptToolCall } from '@/components/app/transcript/SegmentRows'

type Item =
    | { kind: 'tool'; toolCall: TranscriptToolCall }
    | { kind: 'thinking'; id: string }
    | { kind: 'message'; id: string }

const call = (id: string, overrides: Partial<TranscriptToolCall> = {}): Item => ({
    kind: 'tool',
    toolCall: { id, kind: 'command_execution', status: 'completed', title: 'Ran command', command: `cmd ${id}`, filePaths: [], ...overrides },
})
const thinking = (id: string): Item => ({ kind: 'thinking', id })
const message = (id: string): Item => ({ kind: 'message', id })

const classify = (item: Item): TranscriptItemClass =>
    item.kind === 'tool' ? { kind: 'tool_call', toolCall: item.toolCall } : item.kind === 'thinking' ? { kind: 'thinking' } : { kind: 'other' }

const shape = (items: Item[]) => groupTranscript(items, classify).map((block) =>
    block.kind === 'item'
        ? (block.item.kind === 'tool' ? block.item.toolCall.id : block.item.id)
        : `[${block.members.map((member) => member.kind === 'tool_call' ? member.toolCall.id : member.item.kind === 'thinking' ? member.item.id : '?').join(' ')}]`)

describe('groupTranscript', () => {
    it('folds consecutive tool calls and the thinking between them into one group', () => {
        expect(shape([message('m1'), call('a'), thinking('t1'), call('b'), call('c'), message('m2'), call('d')]))
            .toEqual(['m1', '[a t1 b c]', 'm2', '[d]'])
    })

    it('keeps thinking that is not between tool calls as its own block', () => {
        expect(shape([thinking('t0'), call('a'), thinking('t1'), thinking('t2'), message('m'), call('b'), thinking('t3')]))
            .toEqual(['t0', '[a]', 't1', 't2', 'm', '[b]', 't3'])
    })

    it('names a group after its first call and summarizes it', () => {
        const [block] = groupTranscript([
            call('a'), call('b', { status: 'failed' }), call('c', { kind: 'file_change', command: null, filePaths: ['x.ts'] }),
            call('d', { kind: 'dynamic_tool', command: null, status: 'running' }),
        ], classify)
        expect(block.kind).toBe('tool_group')
        if (block.kind !== 'tool_group') return
        expect(block.id).toBe('tool-group:a')
        expect(block.summary).toEqual({ commands: 2, files: 1, tools: 1, failed: 1, running: true, label: 'Ran 2 commands, edited 1 file, used 1 tool' })
    })

    it('returns no blocks for no items', () => {
        expect(groupTranscript([], classify)).toEqual([])
    })
})

describe('summarizeToolGroup', () => {
    const tool = (kind: TranscriptToolCall['kind'], status: TranscriptToolCall['status'] = 'completed'): TranscriptToolCall =>
        ({ id: kind, kind, status, title: kind, filePaths: [] })

    it.each([
        [[tool('command_execution')], 'Ran 1 command'],
        [[tool('command_execution'), tool('command_execution')], 'Ran 2 commands'],
        [[tool('file_change'), tool('file_change')], 'Edited 2 files'],
        [[tool('dynamic_tool')], 'Used 1 tool'],
        [[tool('file_change'), tool('command_execution')], 'Ran 1 command, edited 1 file'],
    ])('labels %j as %s', (calls, label) => {
        expect(summarizeToolGroup(calls).label).toBe(label)
    })

    it('counts failures and reports a running call', () => {
        expect(summarizeToolGroup([tool('command_execution', 'failed'), tool('command_execution', 'failed'), tool('command_execution', 'yielded')]))
            .toMatchObject({ failed: 2, running: false })
        expect(summarizeToolGroup([tool('command_execution', 'running')])).toMatchObject({ failed: 0, running: true })
    })
})
