import { useEffect, useEffectEvent, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { useStore } from '@/store'

const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12
const CONVERSATION_BOTTOM_THRESHOLD_PX = 4

function getHomeSidebarSplitSpace(containerHeight: number) {
    return Math.max(containerHeight - HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT, 0)
}

function clampHomeSidebarPrimaryHeight(height: number, containerHeight: number) {
    if (containerHeight <= 0) {
        return Math.max(height, HOME_SIDEBAR_MIN_PRIMARY_HEIGHT)
    }
    const maxPrimaryHeight = Math.max(
        HOME_SIDEBAR_MIN_PRIMARY_HEIGHT,
        containerHeight - HOME_SIDEBAR_MIN_SECONDARY_HEIGHT - HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT,
    )
    return Math.min(Math.max(height, HOME_SIDEBAR_MIN_PRIMARY_HEIGHT), maxPrimaryHeight)
}

function resolveHomeSidebarPrimaryHeight(containerHeight: number, sidebarPrimarySplitRatio: number | null) {
    const defaultHeight = clampHomeSidebarPrimaryHeight(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT, containerHeight)
    const splitSpace = getHomeSidebarSplitSpace(containerHeight)
    if (splitSpace <= 0) {
        return defaultHeight
    }
    const effectiveRatio = (
        sidebarPrimarySplitRatio !== null && Number.isFinite(sidebarPrimarySplitRatio)
            ? Math.min(Math.max(sidebarPrimarySplitRatio, 0), 1)
            : defaultHeight / splitSpace
    )
    return Math.round(clampHomeSidebarPrimaryHeight(splitSpace * effectiveRatio, containerHeight))
}

function resolveHomeSidebarPrimaryRatio(height: number, containerHeight: number) {
    const splitSpace = getHomeSidebarSplitSpace(containerHeight)
    return splitSpace > 0 ? clampHomeSidebarPrimaryHeight(height, containerHeight) / splitSpace : null
}

export function useHomeSidebarLayout(
    isNarrowViewport: boolean,
    activeProjectPath: string | null,
    activeConversationId: string | null,
) {
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeConversationSessionsById = useStore((state) => state.homeConversationSessionsById)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const updateHomeConversationSession = useStore((state) => state.updateHomeConversationSession)

    const persistedHomeSidebarPrimarySplitRatio = activeProjectPath
        ? (homeProjectSessionsByPath[activeProjectPath]?.sidebarPrimarySplitRatio ?? null)
        : null
    const isConversationPinnedToBottom = activeConversationId
        ? (homeConversationSessionsById[activeConversationId]?.isPinnedToBottom ?? true)
        : true

    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)

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

    const measureHomeSidebarContainerHeight = () => homeSidebarRef.current?.getBoundingClientRect().height || 0

    const syncHomeSidebarPrimaryHeight = useEffectEvent((containerHeight = measureHomeSidebarContainerHeight()) => {
        const nextHeight = resolveHomeSidebarPrimaryHeight(containerHeight, persistedHomeSidebarPrimarySplitRatio)
        setHomeSidebarPrimaryHeight((currentHeight) => (
            currentHeight === nextHeight ? currentHeight : nextHeight
        ))
    })

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

    const setSidebarPrimaryHeight = (nextHeight: number) => {
        const containerHeight = measureHomeSidebarContainerHeight()
        if (containerHeight <= 0) {
            return
        }
        const clampedHeight = clampHomeSidebarPrimaryHeight(nextHeight, containerHeight)
        setHomeSidebarPrimaryHeight(clampedHeight)
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, {
            sidebarPrimarySplitRatio: resolveHomeSidebarPrimaryRatio(clampedHeight, containerHeight),
        })
    }

    const adjustHomeSidebarPrimaryHeight = (delta: number) => {
        setSidebarPrimaryHeight(homeSidebarPrimaryHeight + delta)
    }

    const onHomeSidebarResizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
        if (isNarrowViewport) {
            return
        }
        homeSidebarResizeRef.current = {
            startY: event.clientY,
            startHeight: homeSidebarPrimaryHeight,
        }
        setIsHomeSidebarResizing(true)
        document.body.style.cursor = 'row-resize'
        document.body.style.userSelect = 'none'
        event.preventDefault()
    }

    const onHomeSidebarResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === 'ArrowUp') {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(-24)
            return
        }
        if (event.key === 'ArrowDown') {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(24)
            return
        }
        if (event.key === 'Home') {
            event.preventDefault()
            setSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT)
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            setSidebarPrimaryHeight(Number.POSITIVE_INFINITY)
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            homeSidebarResizeRef.current = null
            setIsHomeSidebarResizing(false)
            setHomeSidebarPrimaryHeight(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
            return
        }
        syncHomeSidebarPrimaryHeight()
    }, [activeProjectPath, isNarrowViewport, persistedHomeSidebarPrimarySplitRatio])

    useEffect(() => {
        if (isNarrowViewport) {
            return
        }
        const node = homeSidebarRef.current
        if (!node) {
            return
        }

        const resizeObserver = new ResizeObserver((entries) => {
            syncHomeSidebarPrimaryHeight(entries[0]?.contentRect.height ?? node.getBoundingClientRect().height)
        })
        resizeObserver.observe(node)

        return () => {
            resizeObserver.disconnect()
        }
    }, [isNarrowViewport])

    useEffect(() => {
        if (isNarrowViewport || !isHomeSidebarResizing) {
            return
        }

        const stopHomeSidebarResize = () => {
            setIsHomeSidebarResizing(false)
            homeSidebarResizeRef.current = null
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }

        const handleHomeSidebarPointerMove = (event: PointerEvent) => {
            const resizeState = homeSidebarResizeRef.current
            if (!resizeState) {
                return
            }
            const nextHeight = resizeState.startHeight + (event.clientY - resizeState.startY)
            setSidebarPrimaryHeight(nextHeight)
        }

        window.addEventListener('pointermove', handleHomeSidebarPointerMove)
        window.addEventListener('pointerup', stopHomeSidebarResize)
        window.addEventListener('pointercancel', stopHomeSidebarResize)
        return () => {
            window.removeEventListener('pointermove', handleHomeSidebarPointerMove)
            window.removeEventListener('pointerup', stopHomeSidebarResize)
            window.removeEventListener('pointercancel', stopHomeSidebarResize)
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }
    }, [isHomeSidebarResizing, isNarrowViewport, activeProjectPath])

    useEffect(() => {
        restoreConversationScrollState()
    }, [activeConversationId])

    return {
        conversationBodyRef,
        homeSidebarRef,
        homeSidebarPrimaryHeight,
        isConversationPinnedToBottom,
        isHomeSidebarResizing: isHomeSidebarResizing && !isNarrowViewport,
        onHomeSidebarResizeKeyDown,
        onHomeSidebarResizePointerDown,
        scrollConversationToBottom,
        syncConversationPinnedState,
    }
}
