import { useEffect, useMemo, useState } from 'react'

import { useStore } from '@/store'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import type { Chat } from '../model/overviewModel'

/**
 * Every registered project's chats, loaded into the shared thread-list cache when the Overview
 * opens; the live feed's summary upserts then keep them current as chats talk and launch.
 */
export function useOverviewChats() {
    const registry = useStore((state) => state.projectRegistry)
    const summaries = useStore((state) => state.homeConversationCache.summariesByProjectPath)
    const setSummaries = useStore((state) => state.setHomeConversationSummaryList)
    const paths = Object.keys(registry).sort().join('\n')
    const [loaded, setLoaded] = useState(false)
    useEffect(() => {
        let disposed = false
        const list = paths.split('\n').filter(Boolean)
        // ponytail: one request per project; add a cross-project chat list if projects grow into the dozens.
        void Promise.all(list.map((path) => (
            fetchProjectConversationListValidated(path)
                .then((chats) => { if (!disposed) setSummaries(path, chats) })
                .catch(() => undefined)
        ))).then(() => {
            if (!disposed) setLoaded(list.length > 0)
        })
        return () => { disposed = true }
    }, [paths, setSummaries])
    const chats = useMemo<Chat[]>(
        () => paths.split('\n').filter(Boolean).flatMap((path) => summaries[path] ?? []),
        [paths, summaries],
    )
    return { chats, loaded }
}
