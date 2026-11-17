import { useEffect, useMemo, useState } from 'react'

import { useStore } from '@/store'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import { applySummarySnapshotToCache } from '@/features/projects/model/projectsHomeState'
import type { Chat } from '../model/overviewModel'

/**
 * Every registered project's chats, loaded into the shared thread-list cache when the Overview
 * opens; the live summary feed then keeps them current as chats talk and launch.
 */
export function useOverviewChats() {
    const registry = useStore((state) => state.projectRegistry)
    const summaries = useStore((state) => state.homeConversationCache.summariesByProjectPath)
    const paths = Object.keys(registry).sort().join('\n')
    const [loaded, setLoaded] = useState(false)
    // The last load failed; the live feed may still fill the list.
    const [failed, setFailed] = useState(false)
    const [attempt, setAttempt] = useState(0)
    useEffect(() => {
        // Fetched before the projects list too: with Spark unreachable, it fails and says so instead of loading forever.
        let disposed = false
        void fetchProjectConversationListValidated(null)
            .then((snapshot) => {
                if (disposed) return
                useStore.getState().commitHomeConversationCache((current) => applySummarySnapshotToCache(current, snapshot, 'http'))
                setFailed(false)
            })
            .catch(() => {
                if (!disposed) setFailed(true)
            })
            .then(() => {
                if (!disposed) setLoaded(true)
            })
        return () => { disposed = true }
    }, [paths, attempt])
    const chats = useMemo<Chat[]>(
        () => paths.split('\n').filter(Boolean).flatMap((path) => summaries[path] ?? []),
        [paths, summaries],
    )
    // Chats arrive keyed by project; until the projects list does too, there are chats to show, not none.
    const waitingOnProjects = !paths && Object.values(summaries).some((list) => list.length > 0)
    return { chats, loaded: loaded && !waitingOnProjects, failed, retry: () => setAttempt((count) => count + 1) }
}
