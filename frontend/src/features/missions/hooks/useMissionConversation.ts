import { useEffect, useState } from 'react'
import { fetchConversationPageValidated } from '@/lib/api/conversationsApi'
import { ApiHttpError } from '@/lib/api/shared'
import { applyConversationPage, type NormalizedConversationRecord } from '@/features/projects/model/projectsHomeState'
import type { Mission } from '../MissionsPanel'

type Loaded = { id: string; record: NormalizedConversationRecord | null; error: string }

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
            // ponytail: the mission log shows its newest page; load older pages like project chats if missions run long.
            return fetchConversationPageValidated(conversationId, project).then(
                page => apply(current => ({
                    id: conversationId,
                    record: applyConversationPage(current?.id === conversationId ? current.record ?? undefined : undefined, page, 'window') ?? null,
                    error: '',
                })),
                (e: unknown) => apply(current => e instanceof ApiHttpError && e.status === 404
                    ? { id: conversationId, record: null, error: '' }
                    : { id: conversationId, record: current?.id === conversationId ? current.record : null, error: e instanceof Error ? e.message : String(e) }),
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
    return { conversationId, record: current?.record ?? null, loading: !current, error: current?.error ?? '', reload: () => setReloads(value => value + 1) }
}
