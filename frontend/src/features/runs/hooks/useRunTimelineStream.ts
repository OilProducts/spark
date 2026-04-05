import { useEffect, useMemo } from 'react'

import { pipelineEventsUrl } from '@/lib/attractorClient'
import { useStore } from '@/store'

import {
    TIMELINE_MAX_ITEMS,
    toTimelineEvent,
} from '../model/timelineModel'

type UseRunTimelineStreamArgs = {
    selectedRunTimelineId: string | null
    manageSync?: boolean
}

const DEFAULT_TIMELINE_SESSION = {
    timelineEvents: [],
    timelineError: null as string | null,
    isTimelineLive: false,
    timelineSequence: 0,
}

export function useRunTimelineStream({
    selectedRunTimelineId,
    manageSync = true,
}: UseRunTimelineStreamArgs) {
    const runDetailSessionsByRunId = useStore((state) => state.runDetailSessionsByRunId)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const session = useMemo(() => {
        if (!selectedRunTimelineId) {
            return DEFAULT_TIMELINE_SESSION
        }
        const current = runDetailSessionsByRunId[selectedRunTimelineId]
        return {
            ...DEFAULT_TIMELINE_SESSION,
            ...(current ?? {}),
        }
    }, [runDetailSessionsByRunId, selectedRunTimelineId])

    useEffect(() => {
        if (!manageSync || !selectedRunTimelineId) {
            return
        }

        const source = new EventSource(pipelineEventsUrl(selectedRunTimelineId))
        source.onopen = () => {
            updateRunDetailSession(selectedRunTimelineId, {
                timelineError: null,
                isTimelineLive: true,
            })
        }
        source.onmessage = (event) => {
            try {
                const payload = JSON.parse(event.data) as unknown
                const currentSession = useStore.getState().runDetailSessionsByRunId[selectedRunTimelineId]
                const currentSequence = currentSession?.timelineSequence ?? 0
                const currentEvents = currentSession?.timelineEvents ?? []
                const timelineEvent = toTimelineEvent(payload, currentSequence)
                if (!timelineEvent) {
                    return
                }
                updateRunDetailSession(selectedRunTimelineId, {
                    timelineEvents: [timelineEvent, ...currentEvents].slice(0, TIMELINE_MAX_ITEMS),
                    timelineSequence: currentSequence + 1,
                })
            } catch {
                // Ignore malformed events.
            }
        }
        source.onerror = () => {
            updateRunDetailSession(selectedRunTimelineId, {
                isTimelineLive: false,
                timelineError: 'Event timeline stream unavailable. Reopen this run to retry.',
            })
        }

        return () => {
            source.close()
            updateRunDetailSession(selectedRunTimelineId, {
                isTimelineLive: false,
            })
        }
    }, [manageSync, selectedRunTimelineId, updateRunDetailSession])

    return {
        isTimelineLive: session.isTimelineLive,
        timelineDroppedCount: Math.max(0, session.timelineSequence - session.timelineEvents.length),
        timelineError: session.timelineError,
        timelineEvents: session.timelineEvents,
    }
}
