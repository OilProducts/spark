import { useEffect, useState } from 'react'
import { fetchConversationSnapshotValidated, type ConversationSnapshotResponse } from '@/lib/api/conversationsApi'
import { ApiHttpError } from '@/lib/api/shared'
import type { Mission } from '../MissionsPanel'

type Loaded = { id: string; snapshot: ConversationSnapshotResponse | null; error: string }

/** Loads the mission's conversation and reloads it as the mission moves. */
export function useMissionConversation(mission: Mission, project: string) {
    // A mission's conversation id is the mission id; a draft's exists once its model is set.
    const conversationId = mission.conversation_id ?? mission.id
    const [loaded, setLoaded] = useState<Loaded | null>(null)
    const [reloads, setReloads] = useState(0)
    const running = mission.status === 'running'
    useEffect(() => {
        let disposed = false
        let issued = 0
        let applied = 0
        // Polls, live events and dependency reloads overlap; only the newest response lands.
        const load = () => {
            const seq = ++issued
            const apply = (next: (current: Loaded | null) => Loaded) => {
                if (disposed || seq < applied) return
                applied = seq
                setLoaded(next)
            }
            return fetchConversationSnapshotValidated(conversationId, project).then(
                snapshot => apply(() => ({ id: conversationId, snapshot, error: '' })),
                (e: unknown) => apply(current => e instanceof ApiHttpError && e.status === 404
                    ? { id: conversationId, snapshot: null, error: '' }
                    : { id: conversationId, snapshot: current?.id === conversationId ? current.snapshot : null, error: e instanceof Error ? e.message : String(e) }),
            )
        }
        void load()
        const onLive = (event: Event) => {
            if ((event as CustomEvent<{ conversationId?: string }>).detail?.conversationId === conversationId) void load()
        }
        window.addEventListener('spark:conversation-live-event', onLive)
        // ponytail: polls while running; subscribe the live stream to mission conversations if this is too chatty.
        const timer = running ? window.setInterval(() => void load(), 2000) : undefined
        return () => { disposed = true; window.removeEventListener('spark:conversation-live-event', onLive); window.clearInterval(timer) }
    }, [conversationId, project, running, reloads, mission.revision, mission.event_seq, mission.cursor, mission.runs?.length])
    // State from another conversation never shows here.
    const current = loaded && loaded.id === conversationId ? loaded : null
    return { conversationId, snapshot: current?.snapshot ?? null, loading: !current, error: current?.error ?? '', reload: () => setReloads(value => value + 1) }
}
