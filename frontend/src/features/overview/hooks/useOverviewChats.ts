import { useEffect, useState } from 'react'

import { useStore } from '@/store'
import { fetchProjectConversationListValidated } from '@/lib/workspaceClient'
import type { Chat } from '../model/overviewModel'

/** Every registered project's chats, loaded when the Overview opens. */
export function useOverviewChats() {
    const registry = useStore((state) => state.projectRegistry)
    const paths = Object.keys(registry).sort().join('\n')
    const [chats, setChats] = useState<Chat[]>([])
    const [loaded, setLoaded] = useState(false)
    useEffect(() => {
        let disposed = false
        // ponytail: one request per project; add a cross-project chat list if projects grow into the dozens.
        void Promise.all(paths.split('\n').filter(Boolean).map((path) => (
            fetchProjectConversationListValidated(path).catch(() => [] as Chat[])
        ))).then((lists) => {
            if (disposed) return
            setChats(lists.flat())
            setLoaded(paths !== "")
        })
        return () => { disposed = true }
    }, [paths])
    return { chats, loaded }
}
