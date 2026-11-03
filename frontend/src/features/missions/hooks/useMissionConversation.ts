import { useEffect, useRef, useState } from 'react'
import { ConversationResetError, fetchConversationChangesValidated, fetchConversationPageValidated } from '@/lib/api/conversationsApi'
import { ApiHttpError } from '@/lib/api/shared'
import { applyConversationBatch, applyConversationPage, type NormalizedConversationRecord } from '@/features/projects/model/projectsHomeState'
import type { Mission } from '../MissionsPanel'

type Loaded = { id: string; record: NormalizedConversationRecord | null; error: string }

/**
 * Loads the mission's conversation and follows it as the mission moves: the newest window first,
 * then the rows changed since its watermark, so older pages loaded on request stay.
 */
export function useMissionConversation(mission: Mission, project: string) {
    // A mission's conversation id is the mission id; a draft's exists once its model is set.
    const conversationId = mission.conversation_id ?? mission.id
    const [loaded, setLoaded] = useState<Loaded | null>(null)
    const [reloads, setReloads] = useState(0)
    const [older, setOlder] = useState<{ id: string; loading: boolean; error: string | null }>({ id: conversationId, loading: false, error: null })
    // The latest applied record, for the next catch-up's cursor.
    const latest = useRef<Loaded | null>(null)
    latest.current = loaded
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
            const known = latest.current?.id === conversationId ? latest.current.record : null
            const window = () => fetchConversationPageValidated(conversationId, project).then(
                page => apply(() => ({ id: conversationId, record: applyConversationPage(undefined, page, 'window') ?? null, error: '' })),
            )
            const request = known && known.watermark !== null
                ? fetchConversationChangesValidated(conversationId, project, known.generation, known.watermark).then(
                    batch => apply(current => {
                        const record = current?.id === conversationId ? current.record ?? undefined : undefined
                        return { id: conversationId, record: applyConversationBatch(record, conversationId, batch, 'live') ?? null, error: '' }
                    }),
                    // Too far behind or another store generation: start over from the newest window.
                    (e: unknown) => {
                        if (e instanceof ConversationResetError) return window()
                        throw e
                    },
                )
                : window()
            return request.catch((e: unknown) => apply(current => e instanceof ApiHttpError && e.status === 404
                ? { id: conversationId, record: null, error: '' }
                : { id: conversationId, record: current?.id === conversationId ? current.record : null, error: e instanceof Error ? e.message : String(e) }))
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
    const record = current?.record ?? null
    const olderState = older.id === conversationId ? older : { id: conversationId, loading: false, error: null }

    const loadOlder = async () => {
        const cursor = record?.olderCursor
        if (!cursor || olderState.loading) return
        setOlder({ id: conversationId, loading: true, error: null })
        try {
            const page = await fetchConversationPageValidated(conversationId, project, cursor)
            // A window loaded meanwhile has its own cursor; this page belongs to the old one.
            setLoaded(currentLoaded => currentLoaded?.id === conversationId && currentLoaded.record?.olderCursor === cursor
                ? { ...currentLoaded, record: applyConversationPage(currentLoaded.record, page, 'older') ?? null }
                : currentLoaded)
            setOlder({ id: conversationId, loading: false, error: null })
        } catch (error) {
            if (error instanceof ConversationResetError) {
                // Turns moved since the window opened: reload the newest window and offer older loading again.
                setLoaded(null)
                setOlder({ id: conversationId, loading: false, error: null })
                setReloads(value => value + 1)
                return
            }
            setOlder({ id: conversationId, loading: false, error: error instanceof Error ? error.message : String(error) })
        }
    }

    return {
        conversationId,
        record,
        loading: !current,
        error: current?.error ?? '',
        reload: () => setReloads(value => value + 1),
        hasOlderMessages: Boolean(record?.olderCursor),
        isLoadingOlderMessages: olderState.loading,
        olderMessagesError: olderState.error,
        loadOlderMessages: loadOlder,
    }
}
