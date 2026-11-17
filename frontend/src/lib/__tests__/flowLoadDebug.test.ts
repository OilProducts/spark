import {
    FLOW_LOAD_DEBUG_QUERY_PARAM,
    FLOW_LOAD_DEBUG_STORAGE_KEY,
    clearFlowLoadDebugTrace,
    isFlowLoadDebugEnabled,
    recordFlowLoadDebug,
    summarizeDiagnosticsForFlowLoadDebug,
} from '@/lib/flowLoadDebug'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('flowLoadDebug', () => {
    beforeEach(() => {
        window.history.replaceState({}, '', '/')
        window.localStorage.clear()
        clearFlowLoadDebugTrace()
        vi.spyOn(console, 'debug').mockImplementation(() => undefined)
    })

    afterEach(() => {
        vi.restoreAllMocks()
        window.history.replaceState({}, '', '/')
        window.localStorage.clear()
        clearFlowLoadDebugTrace()
    })

    it('stays disabled by default', () => {
        expect(isFlowLoadDebugEnabled()).toBe(false)

        recordFlowLoadDebug('preview:request', 'demo.dot', { source: 'load-source' })

        expect(window.__sparkFlowLoadDebug).toEqual([])
        expect(console.debug).not.toHaveBeenCalled()
    })

    it('enables tracing via query param and records entries', () => {
        window.history.replaceState({}, '', `/?${FLOW_LOAD_DEBUG_QUERY_PARAM}=1`)

        recordFlowLoadDebug('preview:request', 'demo.dot', {
            loadId: 1,
            source: 'load-source',
            dotLength: 42,
        })

        expect(isFlowLoadDebugEnabled()).toBe(true)
        expect(window.__sparkFlowLoadDebug).toHaveLength(1)
        expect(window.__sparkFlowLoadDebug?.[0]).toMatchObject({
            event: 'preview:request',
            flowName: 'demo.dot',
            details: {
                loadId: 1,
                source: 'load-source',
                dotLength: 42,
            },
        })
        expect(console.debug).toHaveBeenCalledWith('[flow-load] preview:request', {
            flowName: 'demo.dot',
            loadId: 1,
            source: 'load-source',
            dotLength: 42,
        })
    })

    it('enables tracing via localStorage and caps the ring buffer', () => {
        window.localStorage.setItem(FLOW_LOAD_DEBUG_STORAGE_KEY, '1')

        for (let index = 0; index < 205; index += 1) {
            recordFlowLoadDebug('preview:response', 'demo.dot', { index })
        }

        expect(isFlowLoadDebugEnabled()).toBe(true)
        expect(window.__sparkFlowLoadDebug).toHaveLength(200)
        expect(window.__sparkFlowLoadDebug?.[0]?.details).toEqual({ index: 5 })
        expect(window.__sparkFlowLoadDebug?.[199]?.details).toEqual({ index: 204 })
    })

    it('summarizes diagnostic counts and rule ids', () => {
        expect(
            summarizeDiagnosticsForFlowLoadDebug([
                { severity: 'error', rule_id: 'graph_a', message: 'broken' },
                { severity: 'warning', rule_id: 'graph_b', message: 'careful' },
                { severity: 'warning', rule_id: 'graph_b', message: 'still careful' },
                { severity: 'info', rule_id: 'graph_c', message: 'heads up' },
            ]),
        ).toEqual({
            diagnosticCount: 4,
            errorCount: 1,
            warningCount: 2,
            infoCount: 1,
            ruleIds: ['graph_a', 'graph_b', 'graph_c'],
        })
    })

})
