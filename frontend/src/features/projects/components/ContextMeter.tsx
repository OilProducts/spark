import type { ConversationTurnResponse } from '@/lib/workspaceClient'

/** How full the agent's context is, as adapters record it on `token_usage.context`. */
export interface ContextUsage {
    used_tokens?: number
    window_tokens: number
}

/** The context the latest assistant turn recorded, or null when that turn recorded none. */
export function latestContextUsage(turns: Array<ConversationTurnResponse | undefined>): ContextUsage | null {
    const turn = [...turns].reverse().find((candidate) => candidate?.role === 'assistant')
    const context = turn?.token_usage?.context as Partial<ContextUsage> | undefined
    if (typeof context?.window_tokens !== 'number' || context.window_tokens <= 0) return null
    return {
        window_tokens: context.window_tokens,
        ...(typeof context.used_tokens === 'number' ? { used_tokens: context.used_tokens } : {}),
    }
}

const tokens = (count: number) => (
    count >= 1e6 ? `${+(count / 1e6).toFixed(1)}M` : count >= 1e3 ? `${Math.round(count / 1e3)}k` : `${count}`
)

/** "Context 24k / 1M · 2%"; nothing without data. */
export function ContextMeter({ usage }: { usage: ContextUsage | null }) {
    if (!usage) return null
    const { used_tokens: used, window_tokens: window } = usage
    return (
        <span data-testid="chat-context-meter" className="text-xs tabular-nums text-muted-foreground" title="Context used by the latest agent turn">
            {used === undefined
                ? `Context window ${tokens(window)}`
                : `Context ${tokens(used)} / ${tokens(window)} · ${Math.round((used / window) * 100)}%`}
        </span>
    )
}
