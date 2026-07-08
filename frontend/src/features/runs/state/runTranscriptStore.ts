import { create } from 'zustand'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'

type RunTranscriptStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface RunTranscriptStateEntry {
    segments: RunTranscriptSegment[]
    newestSequence: number
    status: RunTranscriptStatus
    error: string | null
}

interface RunTranscriptStoreState {
    byRunId: Record<string, RunTranscriptStateEntry>
    patchRun: (runId: string, patch: Partial<RunTranscriptStateEntry>) => void
    setSegments: (runId: string, segments: RunTranscriptSegment[], newestSequence: number) => void
    applySegmentUpsert: (runId: string, segment: RunTranscriptSegment) => void
    clearRun: (runId: string) => void
}

function createDefaultRunTranscriptState(): RunTranscriptStateEntry {
    return {
        segments: [],
        newestSequence: 0,
        status: 'idle',
        error: null,
    }
}

const resolveState = (
    byRunId: Record<string, RunTranscriptStateEntry>,
    runId: string,
): RunTranscriptStateEntry => byRunId[runId] ?? createDefaultRunTranscriptState()

export const useRunTranscriptStore = create<RunTranscriptStoreState>()((set) => ({
    byRunId: {},
    patchRun: (runId, patch) =>
        set((state) => ({
            byRunId: {
                ...state.byRunId,
                [runId]: {
                    ...resolveState(state.byRunId, runId),
                    ...patch,
                },
            },
        })),
    setSegments: (runId, segments, newestSequence) =>
        set((state) => ({
            byRunId: {
                ...state.byRunId,
                [runId]: {
                    segments,
                    newestSequence,
                    status: 'ready',
                    error: null,
                },
            },
        })),
    applySegmentUpsert: (runId, segment) =>
        set((state) => {
            const current = resolveState(state.byRunId, runId)
            const index = current.segments.findIndex((existing) => existing.id === segment.id)
            const segments = index >= 0
                ? current.segments.map((existing, position) => (position === index ? segment : existing))
                : [...current.segments, segment]
            return {
                byRunId: {
                    ...state.byRunId,
                    [runId]: {
                        segments,
                        newestSequence: Math.max(current.newestSequence, segment.latest_sequence),
                        status: current.status === 'idle' ? 'ready' : current.status,
                        error: null,
                    },
                },
            }
        }),
    clearRun: (runId) =>
        set((state) => {
            const next = { ...state.byRunId }
            delete next[runId]
            return { byRunId: next }
        }),
}))

export function getRunTranscriptState(runId: string | null): RunTranscriptStateEntry {
    if (!runId) {
        return createDefaultRunTranscriptState()
    }
    return resolveState(useRunTranscriptStore.getState().byRunId, runId)
}
