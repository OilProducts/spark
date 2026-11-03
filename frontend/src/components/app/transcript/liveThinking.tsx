import { useEffect, useState } from 'react'
import { create } from 'zustand'

/** One turn's or node's thinking in progress. Live only: nothing here is stored. */
interface LiveThinking {
    startedAt: number
    estimatedTokens: number | null
    turnId: string | null
}

interface LiveThinkingStore {
    byKey: Record<string, LiveThinking>
    /** Starts or updates `key` while `active`, keeping the start time; otherwise clears it. */
    report: (key: string, active: boolean, estimatedTokens?: number | null, turnId?: string | null) => void
}

export const useLiveThinkingStore = create<LiveThinkingStore>((set) => ({
    byKey: {},
    report: (key, active, estimatedTokens = null, turnId = null) => set((state) => {
        const current = state.byKey[key]
        if (!active) {
            if (!current) return state
            const byKey = { ...state.byKey }
            delete byKey[key]
            return { byKey }
        }
        return {
            byKey: {
                ...state.byKey,
                [key]: {
                    startedAt: current?.startedAt ?? Date.now(),
                    estimatedTokens: estimatedTokens ?? current?.estimatedTokens ?? null,
                    turnId,
                },
            },
        }
    }),
}))

export const conversationThinkingKey = (conversationId: string) => `conversation:${conversationId}`

const formatElapsed = (ms: number) => {
    const seconds = Math.max(0, Math.floor(ms / 1000))
    return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`
}

/**
 * "Thinking · 12s · ~1,200 tokens" while `thinkingKey` is live. The live region
 * stays mounted so assistive tech announces "Thinking" once; the ticking parts are hidden from it.
 */
export function LiveThinkingIndicator({ thinkingKey, testId }: { thinkingKey: string; testId: string }) {
    const thinking = useLiveThinkingStore((state) => state.byKey[thinkingKey])
    const visible = !!thinking
    const [now, setNow] = useState(() => Date.now())
    useEffect(() => {
        if (!visible) return
        setNow(Date.now())
        const timer = window.setInterval(() => setNow(Date.now()), 1000)
        return () => window.clearInterval(timer)
    }, [visible])
    return (
        <div aria-live="polite">
            {visible ? (
                <p data-testid={testId} className="text-xs italic text-muted-foreground">
                    Thinking
                    <span aria-hidden="true">
                        {` · ${formatElapsed(now - thinking.startedAt)}`}
                        {thinking.estimatedTokens === null ? '' : ` · ~${thinking.estimatedTokens.toLocaleString()} tokens`}
                    </span>
                </p>
            ) : null}
        </div>
    )
}
