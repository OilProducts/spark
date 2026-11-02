import { useEffect, useEffectEvent, useLayoutEffect, useRef } from 'react'
import { useStore } from '@/store'

const CONVERSATION_BOTTOM_THRESHOLD_PX = 4

/** Keeps a conversation's scroller at the bottom while pinned, and remembers its position per conversation id. */
export function useScrollFollow(activeConversationId: string | null, hasContent: boolean) {
    const homeConversationSessionsById = useStore((state) => state.homeConversationSessionsById)
    const updateHomeConversationSession = useStore((state) => state.updateHomeConversationSession)
    const isConversationPinnedToBottom = activeConversationId
        ? (homeConversationSessionsById[activeConversationId]?.isPinnedToBottom ?? true)
        : true

    const conversationBodyRef = useRef<HTMLDivElement | null>(null)

    const setConversationLayoutState = (patch: {
        isPinnedToBottom?: boolean
        scrollTop?: number | null
    }) => {
        if (!activeConversationId) {
            return
        }
        const currentConversationSession = useStore.getState().homeConversationSessionsById[activeConversationId] ?? {
            isPinnedToBottom: true,
            scrollTop: null,
        }
        const nextConversationSession = {
            ...(Object.prototype.hasOwnProperty.call(patch, 'isPinnedToBottom')
                ? { isPinnedToBottom: patch.isPinnedToBottom }
                : { isPinnedToBottom: currentConversationSession.isPinnedToBottom }),
            ...(Object.prototype.hasOwnProperty.call(patch, 'scrollTop')
                ? { scrollTop: patch.scrollTop }
                : { scrollTop: currentConversationSession.scrollTop }),
        }
        if (
            nextConversationSession.isPinnedToBottom === currentConversationSession.isPinnedToBottom
            && nextConversationSession.scrollTop === currentConversationSession.scrollTop
        ) {
            return
        }
        updateHomeConversationSession(activeConversationId, nextConversationSession)
    }

    const syncConversationPinnedState = () => {
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight
        setConversationLayoutState({
            isPinnedToBottom: distanceFromBottom <= CONVERSATION_BOTTOM_THRESHOLD_PX,
            scrollTop: node.scrollTop,
        })
    }

    const scrollConversationToBottom = () => {
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        node.scrollTo({
            top: node.scrollHeight,
            behavior: 'smooth',
        })
        setConversationLayoutState({
            isPinnedToBottom: true,
            scrollTop: node.scrollHeight,
        })
    }

    const restoreConversationScrollState = useEffectEvent(() => {
        const node = conversationBodyRef.current
        if (!node || !activeConversationId) {
            return
        }
        const conversationSession = useStore.getState().homeConversationSessionsById[activeConversationId]
        if (!conversationSession) {
            return
        }
        if (conversationSession.isPinnedToBottom) {
            node.scrollTop = node.scrollHeight
            return
        }
        if (typeof conversationSession.scrollTop === 'number') {
            node.scrollTop = conversationSession.scrollTop
        }
    })

    // Restore again once history loads, so a saved scrollTop is not clamped to the loading placeholder.
    useEffect(() => {
        restoreConversationScrollState()
    }, [activeConversationId, hasContent])

    // Follow new content after every commit while pinned.
    useLayoutEffect(() => {
        const node = conversationBodyRef.current
        if (node && isConversationPinnedToBottom) node.scrollTop = node.scrollHeight
    })

    return {
        ref: conversationBodyRef,
        onScroll: syncConversationPinnedState,
        isPinned: isConversationPinnedToBottom,
        scrollToBottom: scrollConversationToBottom,
    }
}
