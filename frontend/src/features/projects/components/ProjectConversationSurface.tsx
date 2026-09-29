import type { FormEvent, KeyboardEvent, ReactNode, RefObject } from 'react'
import { HomeWorkspace } from './HomeWorkspace'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { CodexReconnect } from '@/features/settings/CodexConnectionSettings'
import { isCodexAuthError } from '@/features/settings/services/codexConnection'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { Textarea } from '@/components/ui/textarea'
import type { ConversationChatMode } from '@/lib/workspaceClient'

interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    activeChatMode: ConversationChatMode | null
    defaultModel?: string
    modelSettings: ModelSettings
    inheritedModelSettings?: ModelSettings
    onModelSettingsChange: (value: ModelSettings) => void
    chatModelAvailabilityMessage: string | null
    hasRenderableConversationHistory: boolean
    isConversationPinnedToBottom: boolean
    isNarrowViewport: boolean
    chatDraft: string
    chatSendButtonLabel: string
    isChatInputDisabled: boolean
    isChatSendDisabled: boolean
    panelError: string | null
    conversationBodyRef: RefObject<HTMLDivElement | null>
    historyContent: ReactNode
    onSyncConversationPinnedState: () => void
    onScrollConversationToBottom: () => void
    onStopTurn?: () => void
    onChatComposerSubmit: (event: FormEvent<HTMLFormElement>) => void
    onChatComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void
    onChatDraftChange: (value: string) => void
    modelSettingsSource?: 'workspace' | 'project' | 'conversation'
    onUseModelDefaults?: () => void
}

export function ProjectConversationSurface({
    activeProjectLabel,
    activeProjectPath,
    activeChatMode,
    defaultModel,
    modelSettings,
    inheritedModelSettings,
    onModelSettingsChange,
    chatModelAvailabilityMessage,
    hasRenderableConversationHistory,
    isConversationPinnedToBottom,
    isNarrowViewport,
    chatDraft,
    chatSendButtonLabel,
    isChatInputDisabled,
    isChatSendDisabled,
    panelError,
    conversationBodyRef,
    historyContent,
    onSyncConversationPinnedState,
    onScrollConversationToBottom,
    onStopTurn,
    onChatComposerSubmit,
    onChatComposerKeyDown,
    onChatDraftChange,
    modelSettingsSource,
    onUseModelDefaults,
}: ProjectConversationSurfaceProps) {
    const controlsDisabled = !activeProjectPath || isChatInputDisabled
    return (
        <HomeWorkspace className={isNarrowViewport ? 'space-y-4' : 'h-full'}>
            <Card
                data-testid="project-ai-conversation-surface"
                className={`gap-4 py-4 ${isNarrowViewport ? '' : 'flex h-full min-h-0 flex-col'}`}
            >
                <CardHeader className="gap-1 px-4">
                    <div className="flex items-center justify-between gap-3">
                        <CardTitle className="text-lg font-light">
                            {activeProjectLabel ? `Project Chat - ${activeProjectLabel}` : 'Project Chat'}
                        </CardTitle>
                        {activeProjectPath && activeChatMode ? (
                            <span
                                data-testid="project-active-chat-mode-badge"
                                className="inline-flex items-center rounded-full border border-border px-2 py-1 text-xs font-medium uppercase tracking-wide text-muted-foreground"
                            >
                                {activeChatMode === 'plan' ? 'Plan mode' : 'Chat mode'}
                            </span>
                        ) : null}
                    </div>
                </CardHeader>
                <CardContent className={`space-y-3 px-4 ${isNarrowViewport ? '' : 'flex min-h-0 flex-1 flex-col'}`}>
                {panelError ? (
                    <InlineError data-testid="project-panel-error" dense>
                        {panelError}
                        {isCodexAuthError(panelError) && <CodexReconnect />}
                    </InlineError>
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
                            {chatModelAvailabilityMessage ? (
                                <InlineError data-testid="project-chat-model-availability" dense>{chatModelAvailabilityMessage}</InlineError>
                            ) : null}
                            <Textarea
                                id="project-ai-conversation-input"
                                data-testid="project-ai-conversation-input"
                                value={chatDraft}
                                onChange={(event) => onChatDraftChange(event.target.value)}
                                onKeyDown={onChatComposerKeyDown}
                                aria-label="Message"
                                placeholder="Describe the spec change or requirement you want to work on..."
                                className="h-[clamp(6rem,18vh,12rem)] overflow-y-auto resize-y [field-sizing:fixed]"
                                rows={4}
                            />
                            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                                <p className="text-xs text-muted-foreground">
                                    Press Enter to send. Use Shift+Enter for a new line.
                                </p>
                                <div className="flex flex-wrap items-center justify-end gap-2">
                                    {onStopTurn && <Button type="button" variant="outline" size="sm" onClick={onStopTurn} data-testid="project-chat-stop">Stop</Button>}
                                    {modelSettingsSource && <span className="text-xs text-muted-foreground">{modelSettingsSource === 'conversation' ? 'Conversation override' : modelSettingsSource === 'project' ? 'Project default' : 'Workspace default'}</span>}
                                    {modelSettingsSource === 'conversation' && onUseModelDefaults && <Button type="button" size="sm" variant="ghost" disabled={controlsDisabled} onClick={onUseModelDefaults}>Use defaults</Button>}
                                    <ModelChooser inherited={inheritedModelSettings} value={modelSettings} onChange={onModelSettingsChange}
                                        projectPath={activeProjectPath} inheritLabel="Provider default"
                                        layout="compact" disabled={controlsDisabled} />
                                    {defaultModel && <span className="text-xs text-muted-foreground">Provider default: {defaultModel}</span>}
                                    <Button
                                        data-testid="project-ai-conversation-send-button"
                                        type="submit"
                                        disabled={chatDraft.trim().length === 0 || isChatSendDisabled}
                                        size="sm"
                                        variant="outline"
                                    >
                                        {chatSendButtonLabel}
                                    </Button>
                                </div>
                            </div>
                        </form>
                    </div>
                )}
                </CardContent>
            </Card>
        </HomeWorkspace>
    )
}
