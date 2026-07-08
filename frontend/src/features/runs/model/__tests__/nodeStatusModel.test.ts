import { describe, expect, it } from 'vitest'

import { buildRunNodeStatuses } from '../nodeStatusModel'

describe('buildRunNodeStatuses', () => {
    it('marks checkpoint-completed nodes as success', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['fetch', 'build'],
            currentNodeId: null,
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
        })
        expect(statuses).toEqual({ fetch: 'success', build: 'success' })
    })

    it('overlays live journal statuses over checkpoint state', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['fetch'],
            currentNodeId: null,
            liveNodeStatuses: { fetch: 'failed', deploy: 'running', idlehold: 'idle' },
            gateNodeId: null,
            isRunActive: true,
        })
        expect(statuses.fetch).toBe('failed')
        expect(statuses.deploy).toBe('running')
        expect(statuses.idlehold).toBeUndefined()
    })

    it('marks the current node running only while the run is active and not terminal', () => {
        const active = buildRunNodeStatuses({
            completedNodes: [],
            currentNodeId: 'verify',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: true,
        })
        expect(active.verify).toBe('running')

        const finished = buildRunNodeStatuses({
            completedNodes: [],
            currentNodeId: 'verify',
            liveNodeStatuses: {},
            gateNodeId: null,
            isRunActive: false,
        })
        expect(finished.verify).toBeUndefined()

        const alreadyFailed = buildRunNodeStatuses({
            completedNodes: [],
            currentNodeId: 'verify',
            liveNodeStatuses: { verify: 'failed' },
            gateNodeId: null,
            isRunActive: true,
        })
        expect(alreadyFailed.verify).toBe('failed')
    })

    it('gives a pending human gate the final word', () => {
        const statuses = buildRunNodeStatuses({
            completedNodes: ['approve'],
            currentNodeId: 'approve',
            liveNodeStatuses: { approve: 'running' },
            gateNodeId: 'approve',
            isRunActive: true,
        })
        expect(statuses.approve).toBe('waiting')
    })
})
