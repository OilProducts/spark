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
    useEffect(() => {
        let disposed = false
        if (!paths) return
        void fetchProjectConversationListValidated(null)
            .then((snapshot) => {
                if (!disposed) useStore.getState().commitHomeConversationCache((current) => applySummarySnapshotToCache(current, snapshot, 'http'))
            })
            .catch(() => undefined)
            .then(() => {
                if (!disposed) setLoaded(true)
            })
        return () => { disposed = true }
    }, [paths])
    const chats = useMemo<Chat[]>(
        () => paths.split('\n').filter(Boolean).flatMap((path) => summaries[path] ?? []),
        [paths, summaries],
    )
    return { chats, loaded }
}
