import type { FormEvent, KeyboardEvent, ReactNode, RefObject } from 'react'
import { HomeWorkspace } from '@/components/HomeWorkspace'

interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    hasRenderableConversationHistory: boolean
    isConversationPinnedToBottom: boolean
    isNarrowViewport: boolean
    chatDraft: string
    chatSendButtonLabel: string
    isChatInputDisabled: boolean
    panelError: string | null
    conversationBodyRef: RefObject<HTMLDivElement | null>
    historyContent: ReactNode
    onSyncConversationPinnedState: () => void
    onScrollConversationToBottom: () => void
    onChatComposerSubmit: (event: FormEvent<HTMLFormElement>) => void
    onChatComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void
    onChatDraftChange: (value: string) => void
}

export function ProjectConversationSurface({
    activeProjectLabel,
    activeProjectPath,
    hasRenderableConversationHistory,
    isConversationPinnedToBottom,
    isNarrowViewport,
    chatDraft,
    chatSendButtonLabel,
    isChatInputDisabled,
    panelError,
    conversationBodyRef,
    historyContent,
    onSyncConversationPinnedState,
    onScrollConversationToBottom,
    onChatComposerSubmit,
    onChatComposerKeyDown,
    onChatDraftChange,
}: ProjectConversationSurfaceProps) {
    return (
        <HomeWorkspace className={isNarrowViewport ? 'space-y-4' : 'h-full'}>
            <div
                data-testid="project-ai-conversation-surface"
                className={`rounded-md border border-border bg-card p-4 shadow-sm ${isNarrowViewport ? '' : 'flex h-full min-h-0 flex-col'}`}
            >
                <p className="mb-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    {activeProjectLabel ? `Project Chat - ${activeProjectLabel}` : 'Project Chat'}
                </p>
                {panelError ? (
                    <p
                        data-testid="project-panel-error"
                        className="mb-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                    >
                        {panelError}
                    </p>
                ) : null}
                {!activeProjectPath ? (
                    <p className={`rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground ${isNarrowViewport ? '' : 'flex flex-1 items-center'}`}>
                        Select an active project to begin chatting.
                    </p>
                ) : (
                    <div className="flex min-h-0 flex-1 flex-col gap-3">
                        <div
                            ref={conversationBodyRef}
                            data-testid="project-ai-conversation-body"
                            onScroll={onSyncConversationPinnedState}
                            className={`flex min-h-0 flex-1 flex-col gap-3 ${isNarrowViewport ? '' : 'overflow-y-auto pr-1'}`}
                        >
                            {historyContent}
                        </div>
                        {!isConversationPinnedToBottom && hasRenderableConversationHistory ? (
                            <div className="flex justify-end">
                                <button
                                    type="button"
                                    data-testid="project-ai-conversation-jump-to-bottom"
                                    onClick={onScrollConversationToBottom}
                                    className="rounded border border-border bg-background/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                >
                                    Jump to bottom
                                </button>
                            </div>
                        ) : null}
                        <form
                            data-testid="project-ai-conversation-composer"
                            onSubmit={onChatComposerSubmit}
                            className="shrink-0 space-y-2 pt-1"
                        >
                            <textarea
                                id="project-ai-conversation-input"
                                data-testid="project-ai-conversation-input"
                                value={chatDraft}
                                onChange={(event) => onChatDraftChange(event.target.value)}
                                onKeyDown={onChatComposerKeyDown}
                                aria-label="Message"
                                placeholder="Describe the spec change or requirement you want to work on..."
                                rows={4}
                                className="w-full rounded border border-border bg-background px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                            <div className="flex items-center justify-between gap-2">
                                <p className="text-[11px] text-muted-foreground">
                                    Press Enter to send. Use Shift+Enter for a new line.
                                </p>
                                <button
                                    data-testid="project-ai-conversation-send-button"
                                    type="submit"
                                    disabled={chatDraft.trim().length === 0 || isChatInputDisabled}
                                    className="rounded border border-border px-3 py-1.5 text-xs font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                >
                                    {chatSendButtonLabel}
                                </button>
                            </div>
                        </form>
                    </div>
                )}
            </div>
        </HomeWorkspace>
    )
}
