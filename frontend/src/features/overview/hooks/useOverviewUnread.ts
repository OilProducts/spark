import { useSyncExternalStore } from 'react'

import { useStore } from '@/store'
import type { AttentionItem } from '@/lib/api/attentionApi'
import { hasUnread, readSeenAttention, readSeenFinished, subscribeSeen } from '../model/overviewModel'

/** Whether the Overview icon shows its dot: something needs you or finished that the Overview has not shown, and you are elsewhere. */
export function useOverviewUnread(attention: AttentionItem[]) {
    const viewMode = useStore((state) => state.viewMode)
    const runs = useStore((state) => state.runsListSession.runs)
    const missions = useStore((state) => state.missionBoard)
    const seenAttention = useSyncExternalStore(subscribeSeen, readSeenAttention)
    const seenFinished = useSyncExternalStore(subscribeSeen, readSeenFinished)
    // eslint-disable-next-line react-hooks/purity -- the first-visit window only; the bar re-renders on run, mission and view changes
    return viewMode !== 'overview' && hasUnread(attention, runs, missions, Date.now(), seenAttention, seenFinished)
}
