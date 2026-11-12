import { describe, expect, it } from 'vitest'

import { READABLE_ZOOM, readableViewport } from '../graphFraming'

const padding = { top: 10, right: 10, bottom: 10, left: 10 }

describe('readableViewport', () => {
    it('centres a graph that fits, never magnifying past 1x', () => {
        expect(readableViewport({ x: 0, y: 0, width: 100, height: 100 }, { width: 520, height: 320 }, padding, null))
            .toEqual({ x: 210, y: 110, zoom: 1 })
    })

    it('shows a graph whole while its labels stay readable', () => {
        const viewport = readableViewport({ x: 0, y: 0, width: 400, height: 900 }, { width: 420, height: 740 }, padding, null)
        expect(viewport.zoom).toBeCloseTo(0.8)
        expect(viewport.y).toBeCloseTo(10)
    })

    it('opens a graph too big to read whole at the top, at a readable zoom', () => {
        const viewport = readableViewport({ x: 0, y: 0, width: 400, height: 2000 }, { width: 420, height: 740 }, padding, null)
        expect(viewport).toEqual({ x: (420 - 400 * READABLE_ZOOM) / 2, y: 10, zoom: READABLE_ZOOM })
    })

    it('centres on the focus without showing canvas past the graph edge', () => {
        const bounds = { x: 0, y: 0, width: 400, height: 2000 }
        expect(readableViewport(bounds, { width: 420, height: 740 }, padding, { x: 200, y: 1000 }).y).toBeCloseTo(370 - 1000 * READABLE_ZOOM)
        // A focus near the bottom stops with the graph's last row at the bottom padding.
        expect(readableViewport(bounds, { width: 420, height: 740 }, padding, { x: 200, y: 1990 }).y).toBeCloseTo(730 - 2000 * READABLE_ZOOM)
    })
})
