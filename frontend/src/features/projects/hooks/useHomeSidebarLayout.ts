import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { useStore } from '@/store'

const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12
const CONVERSATION_BOTTOM_THRESHOLD_PX = 24

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

function clampHomeSidebarPrimaryRatio(ratio: number) {
    if (!Number.isFinite(ratio)) {
        return null
    }
    return Math.min(Math.max(ratio, 0), 1)
}

function getDefaultHomeSidebarPrimaryRatio(containerHeight: number) {
    const splitSpace = getHomeSidebarSplitSpace(containerHeight)
    if (splitSpace <= 0) {
        return null
    }
    return clampHomeSidebarPrimaryRatio(
        clampHomeSidebarPrimaryHeight(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT, containerHeight) / splitSpace,
    )
}

function resolveHomeSidebarPrimaryHeight(sidebarPrimarySplitRatio: number | null, containerHeight: number) {
    if (containerHeight <= 0) {
        return DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT
    }
    const effectiveRatio = sidebarPrimarySplitRatio ?? getDefaultHomeSidebarPrimaryRatio(containerHeight)
    const splitSpace = getHomeSidebarSplitSpace(containerHeight)
    if (effectiveRatio === null || splitSpace <= 0) {
        return clampHomeSidebarPrimaryHeight(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT, containerHeight)
    }
    return Math.round(
        clampHomeSidebarPrimaryHeight(splitSpace * effectiveRatio, containerHeight),
    )
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
    const [homeSidebarContainerHeight, setHomeSidebarContainerHeight] = useState(0)

    const effectiveIsHomeSidebarResizing = isHomeSidebarResizing && !isNarrowViewport
    const effectiveHomeSidebarContainerHeight = homeSidebarContainerHeight
        || homeSidebarRef.current?.getBoundingClientRect().height
        || 0
    const homeSidebarPrimaryHeight = resolveHomeSidebarPrimaryHeight(
        persistedHomeSidebarPrimarySplitRatio,
        effectiveHomeSidebarContainerHeight,
    )

    const setConversationLayoutState = (patch: {
        isPinnedToBottom?: boolean
        scrollTop?: number | null
    }) => {
        if (!activeConversationId) {
            return
        }
        const currentConversationSession = homeConversationSessionsById[activeConversationId] ?? {
            isPinnedToBottom: true,
            scrollTop: null,
        }
        updateHomeConversationSession(activeConversationId, {
            ...(Object.prototype.hasOwnProperty.call(patch, 'isPinnedToBottom')
                ? { isPinnedToBottom: patch.isPinnedToBottom }
                : { isPinnedToBottom: currentConversationSession.isPinnedToBottom }),
            ...(Object.prototype.hasOwnProperty.call(patch, 'scrollTop')
                ? { scrollTop: patch.scrollTop }
                : { scrollTop: currentConversationSession.scrollTop }),
        })
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

    const readHomeSidebarContainerHeight = () => {
        const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
        setHomeSidebarContainerHeight((currentHeight) => (
            currentHeight === containerHeight ? currentHeight : containerHeight
        ))
        return containerHeight
    }

    const setSidebarPrimaryHeight = (nextHeight: number, containerHeight: number) => {
        if (!activeProjectPath) {
            return
        }
        const splitSpace = getHomeSidebarSplitSpace(containerHeight)
        const clampedHeight = clampHomeSidebarPrimaryHeight(nextHeight, containerHeight)
        updateHomeProjectSession(activeProjectPath, {
            sidebarPrimarySplitRatio: splitSpace > 0
                ? clampHomeSidebarPrimaryRatio(clampedHeight / splitSpace)
                : null,
        })
    }

    const adjustHomeSidebarPrimaryHeight = (delta: number) => {
        const containerHeight = readHomeSidebarContainerHeight()
        if (containerHeight <= 0) {
            return
        }
        setSidebarPrimaryHeight(homeSidebarPrimaryHeight + delta, containerHeight)
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
            const containerHeight = readHomeSidebarContainerHeight()
            if (containerHeight <= 0) {
                return
            }
            setSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT, containerHeight)
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            const containerHeight = readHomeSidebarContainerHeight()
            if (containerHeight <= 0) {
                return
            }
            setSidebarPrimaryHeight(containerHeight, containerHeight)
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            homeSidebarResizeRef.current = null
            setIsHomeSidebarResizing(false)
            setHomeSidebarContainerHeight(0)
            return
        }
        const node = homeSidebarRef.current
        if (!node) {
            return
        }

        readHomeSidebarContainerHeight()

        const resizeObserver = new ResizeObserver(() => {
            readHomeSidebarContainerHeight()
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
            const containerHeight = readHomeSidebarContainerHeight()
            if (!resizeState || containerHeight <= 0) {
                return
            }
            const nextHeight = resizeState.startHeight + (event.clientY - resizeState.startY)
            setSidebarPrimaryHeight(nextHeight, containerHeight)
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
        const node = conversationBodyRef.current
        if (!node || !activeConversationId) {
            return
        }
        const conversationSession = homeConversationSessionsById[activeConversationId]
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
    }, [activeConversationId, homeConversationSessionsById])

    return {
        conversationBodyRef,
        homeSidebarRef,
        homeSidebarPrimaryHeight,
        isConversationPinnedToBottom,
        isHomeSidebarResizing: effectiveIsHomeSidebarResizing,
        onHomeSidebarResizeKeyDown,
        onHomeSidebarResizePointerDown,
        scrollConversationToBottom,
        syncConversationPinnedState,
    }
}
