import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'

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

export function useHomeSidebarLayout(isNarrowViewport: boolean, pinResetKey: string | null) {
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const [isConversationPinnedToBottom, setIsConversationPinnedToBottom] = useState(true)
    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)

    const syncConversationPinnedState = () => {
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight
        setIsConversationPinnedToBottom(distanceFromBottom <= CONVERSATION_BOTTOM_THRESHOLD_PX)
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
        setIsConversationPinnedToBottom(true)
    }

    const adjustHomeSidebarPrimaryHeight = (delta: number) => {
        const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
        if (containerHeight <= 0) {
            return
        }
        setHomeSidebarPrimaryHeight((current) => clampHomeSidebarPrimaryHeight(current + delta, containerHeight))
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
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT, containerHeight))
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(containerHeight, containerHeight))
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            setIsHomeSidebarResizing(false)
            homeSidebarResizeRef.current = null
            return
        }

        const syncSidebarHeight = () => {
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight((current) => clampHomeSidebarPrimaryHeight(current, containerHeight))
        }

        syncSidebarHeight()
        window.addEventListener('resize', syncSidebarHeight)
        return () => {
            window.removeEventListener('resize', syncSidebarHeight)
        }
    }, [isNarrowViewport])

    useEffect(() => {
        if (!isHomeSidebarResizing) {
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
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(nextHeight, containerHeight))
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
    }, [isHomeSidebarResizing])

    useEffect(() => {
        setIsConversationPinnedToBottom(true)
    }, [pinResetKey])

    return {
        conversationBodyRef,
        homeSidebarRef,
        homeSidebarPrimaryHeight,
        isConversationPinnedToBottom,
        isHomeSidebarResizing,
        onHomeSidebarResizeKeyDown,
        onHomeSidebarResizePointerDown,
        scrollConversationToBottom,
        syncConversationPinnedState,
    }
}
