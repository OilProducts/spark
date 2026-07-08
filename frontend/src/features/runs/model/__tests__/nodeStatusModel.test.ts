import { describe, expect, it } from 'vitest'

import { buildRunNodeStatuses } from '../nodeStatusModel'

describe('buildRunNodeStatuses', () => {
    it('marks checkpoint-completed nodes as success', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['fetch', 'build'],
            nodeOutcomes: {},
            currentNodeId: null,
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
            runStatus: 'completed',
        })
        expect(statuses).toEqual({ fetch: 'success', build: 'success' })
    })

    it('overlays live journal statuses over checkpoint state', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['fetch'],
            nodeOutcomes: {},
            currentNodeId: null,
            liveNodeStatuses: { fetch: 'failed', deploy: 'running', idlehold: 'idle' },
            gateNodeId: null,
            isRunActive: true,
            runStatus: 'running',
        })
        expect(statuses.fetch).toBe('failed')
        expect(statuses.deploy).toBe('running')
        expect(statuses.idlehold).toBeUndefined()
    })

    it('marks the current node running only while the run is active and not terminal', () => {
        const active = buildRunNodeStatuses({
            completedNodes: [],
            nodeOutcomes: {},
            currentNodeId: 'verify',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: true,
            runStatus: 'running',
        })
        expect(active.verify).toBe('running')

        const finished = buildRunNodeStatuses({
            completedNodes: [],
            nodeOutcomes: {},
            currentNodeId: 'verify',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
            runStatus: 'completed',
        })
        expect(finished.verify).toBeUndefined()

        const alreadyFailed = buildRunNodeStatuses({
            completedNodes: [],
            nodeOutcomes: {},
            currentNodeId: 'verify',
            liveNodeStatuses: { verify: 'failed' },
            gateNodeId: null,
            isRunActive: true,
            runStatus: 'running',
        })
        expect(alreadyFailed.verify).toBe('failed')
    })

    it('marks failed checkpoint outcomes failed even though they sit in completed_nodes', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['load', 'transform'],
            nodeOutcomes: { load: 'success', transform: 'fail' },
            currentNodeId: 'transform',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
            runStatus: 'failed',
        })
        expect(statuses.load).toBe('success')
        expect(statuses.transform).toBe('failed')
    })

    it('marks the failed run current node failed without a checkpoint outcome', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['load', 'transform'],
            nodeOutcomes: {},
            currentNodeId: 'transform',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
            runStatus: 'failed',
        })
        expect(statuses.transform).toBe('failed')
        expect(statuses.load).toBe('success')
    })

    it('gives a pending human gate the final word', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['approve'],
            nodeOutcomes: {},
            currentNodeId: 'approve',
            liveNodeStatuses: { approve: 'running' },
            gateNodeId: 'approve',
            isRunActive: true,
            runStatus: 'waiting',
        })
        expect(statuses.approve).toBe('waiting')
    })
})
