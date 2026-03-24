import type { FormEvent, KeyboardEvent, ReactNode, RefObject } from 'react'
import { HomeWorkspace } from '@/components/HomeWorkspace'
import { Button, EmptyState, InlineNotice, Panel, PanelContent, PanelHeader, PanelTitle, Textarea } from '@/ui'

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
            <Panel
                data-testid="project-ai-conversation-surface"
                className={`${isNarrowViewport ? '' : 'flex h-full min-h-0 flex-col'}`}
            >
                <PanelHeader>
                    <PanelTitle>{activeProjectLabel ? `Project Chat - ${activeProjectLabel}` : 'Project Chat'}</PanelTitle>
                </PanelHeader>
                <PanelContent className={`space-y-3 ${isNarrowViewport ? '' : 'flex min-h-0 flex-1 flex-col'}`}>
                {panelError ? (
                    <InlineNotice data-testid="project-panel-error" tone="error" className="text-xs">
                        {panelError}
                    </InlineNotice>
                ) : null}
                {!activeProjectPath ? (
                    <EmptyState
                        className={isNarrowViewport ? '' : 'flex flex-1 items-center'}
                        description="Select an active project to begin chatting."
                    />
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
                                <Button
                                    type="button"
                                    data-testid="project-ai-conversation-jump-to-bottom"
                                    onClick={onScrollConversationToBottom}
                                    variant="outline"
                                    size="xs"
                                >
                                    Jump to bottom
                                </Button>
                            </div>
                        ) : null}
                        <form
                            data-testid="project-ai-conversation-composer"
                            onSubmit={onChatComposerSubmit}
                            className="shrink-0 space-y-2 pt-1"
                        >
                            <Textarea
                                id="project-ai-conversation-input"
                                data-testid="project-ai-conversation-input"
                                value={chatDraft}
                                onChange={(event) => onChatDraftChange(event.target.value)}
                                onKeyDown={onChatComposerKeyDown}
                                aria-label="Message"
                                placeholder="Describe the spec change or requirement you want to work on..."
                                rows={4}
                            />
                            <div className="flex items-center justify-between gap-2">
                                <p className="text-[11px] text-muted-foreground">
                                    Press Enter to send. Use Shift+Enter for a new line.
                                </p>
                                <Button
                                    data-testid="project-ai-conversation-send-button"
                                    type="submit"
                                    disabled={chatDraft.trim().length === 0 || isChatInputDisabled}
                                    size="sm"
                                    variant="outline"
                                >
                                    {chatSendButtonLabel}
                                </Button>
                            </div>
                        </form>
                    </div>
                )}
                </PanelContent>
            </Panel>
        </HomeWorkspace>
    )
}
