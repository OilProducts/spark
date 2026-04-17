import type { FormEvent, KeyboardEvent, ReactNode, RefObject } from 'react'
import { HomeWorkspace } from './HomeWorkspace'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { Textarea } from '@/components/ui/textarea'
import type { ConversationChatMode } from '@/lib/workspaceClient'
interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    activeChatMode: ConversationChatMode | null
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
    activeChatMode,
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
            <Card
                data-testid="project-ai-conversation-surface"
                className={`gap-4 py-4 ${isNarrowViewport ? '' : 'flex h-full min-h-0 flex-col'}`}
            >
                <CardHeader className="gap-1 px-4">
                    <div className="flex items-center justify-between gap-3">
                        <CardTitle className="text-sm">
                            {activeProjectLabel ? `Project Chat - ${activeProjectLabel}` : 'Project Chat'}
                        </CardTitle>
                        {activeProjectPath && activeChatMode ? (
                            <span
                                data-testid="project-active-chat-mode-badge"
                                className="inline-flex items-center rounded-full border border-border/70 bg-muted/40 px-2 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground"
                            >
                                {activeChatMode === 'plan' ? 'Plan mode' : 'Chat mode'}
                            </span>
                        ) : null}
                    </div>
                </CardHeader>
                <CardContent className={`space-y-3 px-4 ${isNarrowViewport ? '' : 'flex min-h-0 flex-1 flex-col'}`}>
                {panelError ? (
                    <Alert
                        data-testid="project-panel-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                    >
                        <AlertDescription className="text-inherit">{panelError}</AlertDescription>
                    </Alert>
                ) : null}
                {!activeProjectPath ? (
                    <Empty className={`text-sm text-muted-foreground ${isNarrowViewport ? '' : 'flex flex-1 items-center'}`}>
                        <EmptyHeader>
                            <EmptyDescription>
                                Choose or add a project from the navbar to begin chatting.
                            </EmptyDescription>
                        </EmptyHeader>
                    </Empty>
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
                </CardContent>
            </Card>
        </HomeWorkspace>
    )
}
