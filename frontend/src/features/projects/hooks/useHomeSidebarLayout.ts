import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { useStore } from '@/store'

const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12
const CONVERSATION_BOTTOM_THRESHOLD_PX = 24

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

export function useHomeSidebarLayout(
    isNarrowViewport: boolean,
    activeProjectPath: string | null,
    activeConversationId: string | null,
) {
    const homeProjectSessionsByPath = useStore((state) => state.homeProjectSessionsByPath)
    const homeConversationSessionsById = useStore((state) => state.homeConversationSessionsById)
    const updateHomeProjectSession = useStore((state) => state.updateHomeProjectSession)
    const updateHomeConversationSession = useStore((state) => state.updateHomeConversationSession)

    const homeSidebarPrimaryHeight = activeProjectPath
        ? (homeProjectSessionsByPath[activeProjectPath]?.sidebarPrimaryHeight ?? DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
        : DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT
    const isConversationPinnedToBottom = activeConversationId
        ? (homeConversationSessionsById[activeConversationId]?.isPinnedToBottom ?? true)
        : true

    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)

    const effectiveIsHomeSidebarResizing = isHomeSidebarResizing && !isNarrowViewport

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

    const setSidebarPrimaryHeight = (nextHeight: number) => {
        if (!activeProjectPath) {
            return
        }
        updateHomeProjectSession(activeProjectPath, { sidebarPrimaryHeight: nextHeight })
    }

    const adjustHomeSidebarPrimaryHeight = (delta: number) => {
        const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
        if (containerHeight <= 0) {
            return
        }
        setSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(homeSidebarPrimaryHeight + delta, containerHeight))
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
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT, containerHeight))
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(containerHeight, containerHeight))
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            homeSidebarResizeRef.current = null
            setIsHomeSidebarResizing(false)
            return
        }

        const syncSidebarHeight = () => {
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(homeSidebarPrimaryHeight, containerHeight))
        }

        syncSidebarHeight()
        window.addEventListener('resize', syncSidebarHeight)
        return () => {
            window.removeEventListener('resize', syncSidebarHeight)
        }
    }, [homeSidebarPrimaryHeight, isNarrowViewport, activeProjectPath])

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
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (!resizeState || containerHeight <= 0) {
                return
            }
            const nextHeight = resizeState.startHeight + (event.clientY - resizeState.startY)
            setSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(nextHeight, containerHeight))
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
