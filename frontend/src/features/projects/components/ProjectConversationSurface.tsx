import type { ReactNode } from 'react'
import { Folder } from 'lucide-react'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { ClaudeCodeReconnect } from '@/features/settings/ClaudeCodeConnectionSettings'
import { CodexReconnect } from '@/features/settings/CodexConnectionSettings'
import { isClaudeCodeAuthError } from '@/features/settings/services/claudeCodeConnection'
import { isCodexAuthError } from '@/features/settings/services/codexConnection'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import { ShowThinkingToggle } from '@/components/app/transcript/SegmentRows'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import type { ConversationChatMode } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { cn } from '@/lib/utils'
import { formatConversationAgeShort, formatConversationTimestamp } from '../model/projectsHomeState'
import type { useScrollFollow } from '../hooks/useScrollFollow'
import type { ComposerAttachment } from '../model/types'
import { ChatPane } from './ChatPane'
import { ContextMeter, type ContextUsage } from './ContextMeter'

interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    activeConversationTitle?: string | null
    activeConversationHandle?: string | null
    activeConversationUpdatedAt?: string | null
    activeChatMode: ConversationChatMode | null
    /** The latest assistant turn's context occupancy, for the meter. */
    contextUsage?: ContextUsage | null
    onChatModeChange?: (mode: ConversationChatMode) => void
    /** Whether the transcript has thinking to reveal; the toggle is omitted otherwise. */
    hasThinking: boolean
    showThinking: boolean
    onShowThinkingChange: (next: boolean) => void
    defaultModel?: string
    modelSettings: ModelSettings
    inheritedModelSettings?: ModelSettings
    onModelSettingsChange: (value: ModelSettings) => void
    isModelSettingsLoading?: boolean
    chatModelAvailabilityMessage: string | null
    hasRenderableConversationHistory: boolean
    follow: ReturnType<typeof useScrollFollow>
    isNarrowViewport: boolean
    chatDraft: string
    attachments?: ComposerAttachment[]
    onAddAttachments?: (files: File[]) => void
    onRetryAttachment?: (localId: string) => void
    onRemoveAttachment?: (localId: string) => void
    isAttachmentSendPending?: boolean
    /** An upload is pending or failed, so the message cannot go yet. */
    isUploadBlocking?: boolean
    chatSendButtonLabel: string
    isChatInputDisabled: boolean
    isChatSendDisabled: boolean
    panelError: string | null
    historyContent: ReactNode
    onStopTurn?: () => void
    onChatComposerSubmit: () => void
    onChatDraftChange: (value: string) => void
    modelSettingsSource?: 'workspace' | 'project' | 'role' | 'conversation'
    onUseModelDefaults?: () => void
}

const MODEL_SOURCE_LABELS = {
    conversation: 'Conversation override',
    role: 'Role default',
    project: 'Project default',
    workspace: 'Workspace default',
} as const

/** The chat sheet: header, scrolling transcript and the composer. ViewLayout supplies the sheet itself. */
export function ProjectConversationSurface({
    activeProjectLabel,
    activeProjectPath,
    activeConversationTitle = null,
    activeConversationHandle = null,
    activeConversationUpdatedAt = null,
    activeChatMode,
    contextUsage = null,
    onChatModeChange,
    hasThinking,
    showThinking,
    onShowThinkingChange,
    defaultModel,
    modelSettings,
    inheritedModelSettings,
    onModelSettingsChange,
    isModelSettingsLoading = false,
    chatModelAvailabilityMessage,
    hasRenderableConversationHistory,
    follow,
    isNarrowViewport,
    chatDraft,
    attachments = [],
    onAddAttachments,
    onRetryAttachment = () => undefined,
    onRemoveAttachment = () => undefined,
    isAttachmentSendPending = false,
    isUploadBlocking = false,
    chatSendButtonLabel,
    isChatInputDisabled,
    isChatSendDisabled,
    panelError,
    historyContent,
    onStopTurn,
    onChatComposerSubmit,
    onChatDraftChange,
    modelSettingsSource,
    onUseModelDefaults,
}: ProjectConversationSurfaceProps) {
    const controlsDisabled = !activeProjectPath || isChatInputDisabled
    const hasProject = Boolean(activeProjectPath && activeProjectLabel)
    return (
        <div
            data-testid="project-ai-conversation-surface"
            className={cn('flex flex-col', !isNarrowViewport && 'h-full min-h-0')}
        >
            <header className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 border-b border-border px-6 py-4">
                <div className="min-w-0">
                    <h1 data-testid="chat-title" className="truncate text-xl font-semibold text-foreground">
                        {activeConversationTitle || 'Chat'}
                    </h1>
                    {hasProject ? (
                        <p className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
                            {activeConversationHandle ? (
                                <>
                                    <span data-testid="chat-handle" className="font-mono">{activeConversationHandle}</span>
                                    <span aria-hidden="true">·</span>
                                </>
                            ) : null}
                            <button
                                type="button"
                                data-testid="chat-project-link"
                                title={activeProjectPath ?? undefined}
                                onClick={() => activeProjectPath && useStore.getState().openProjectPage(activeProjectPath)}
                                className="min-w-0 truncate outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                            >
                                {activeProjectLabel}
                            </button>
                            {activeConversationUpdatedAt ? (
                                <>
                                    <span aria-hidden="true">·</span>
                                    <span className="shrink-0" title={formatConversationTimestamp(activeConversationUpdatedAt)}>
                                        {formatConversationAgeShort(activeConversationUpdatedAt)}
                                    </span>
                                </>
                            ) : null}
                        </p>
                    ) : null}
                </div>
                <div className="flex items-center gap-2">
                    {hasThinking ? <ShowThinkingToggle pressed={showThinking} onPressedChange={onShowThinkingChange} /> : null}
                    {activeProjectPath && activeChatMode ? (
                        <NativeSelect
                            data-testid="project-active-chat-mode-badge"
                            aria-label="Mode"
                            size="sm"
                            value={activeChatMode}
                            // Same guard as the /plan command: no switching mid-turn or before the model list is ready.
                            disabled={controlsDisabled || isChatSendDisabled || !onChatModeChange}
                            onChange={(event) => onChatModeChange?.(event.target.value === 'plan' ? 'plan' : 'chat')}
                            className="rounded-lg"
                        >
                            <NativeSelectOption value="chat">Chat</NativeSelectOption>
                            <NativeSelectOption value="plan">Plan</NativeSelectOption>
                        </NativeSelect>
                    ) : null}
                </div>
            </header>
            <div className={cn('flex flex-col', !isNarrowViewport && 'min-h-0 flex-1')}>
                {panelError ? (
                    <InlineError data-testid="project-panel-error" dense className="mx-6 mt-4">
                        {panelError}
                        {isCodexAuthError(panelError) && <CodexReconnect />}
                        {isClaudeCodeAuthError(panelError) && <ClaudeCodeReconnect />}
                    </InlineError>
                ) : null}
                {!activeProjectPath ? (
                    <div className="px-6 py-6">
                        <Empty>
                            <EmptyHeader>
                                <EmptyDescription>
                                    Pick a chat, or start a new one.
                                </EmptyDescription>
                            </EmptyHeader>
                        </Empty>
                    </div>
                ) : (
                    <ChatPane
                        follow={follow}
                        scrolls={!isNarrowViewport}
                        hasContent={hasRenderableConversationHistory}
                        notice={chatModelAvailabilityMessage ? (
                            <InlineError data-testid="project-chat-model-availability" dense>{chatModelAvailabilityMessage}</InlineError>
                        ) : null}
                        composer={{
                            placeholder: 'Reply, or describe a change to work on',
                            draft: chatDraft,
                            onDraftChange: onChatDraftChange,
                            onSubmit: onChatComposerSubmit,
                            inputDisabled: false,
                            sendDisabled: isChatSendDisabled,
                            sendLabel: chatSendButtonLabel,
                            attachments: onAddAttachments ? {
                                items: attachments,
                                onAdd: onAddAttachments,
                                onRetry: onRetryAttachment,
                                onRemove: onRemoveAttachment,
                                isSending: isAttachmentSendPending,
                                isUploadBlocking,
                                disabled: controlsDisabled,
                            } : undefined,
                            footer: (
                                <>
                                    <p data-testid="chat-composer-project" className="inline-flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                                        <Folder aria-hidden="true" className="size-3.5 shrink-0" />
                                        <span className="truncate">Runs and missions started here go to <span className="text-foreground">{activeProjectLabel}</span></span>
                                    </p>
                                    {modelSettingsSource ? <span className="text-xs text-muted-foreground">{MODEL_SOURCE_LABELS[modelSettingsSource]}</span> : null}
                                    {defaultModel ? <span className="text-xs text-muted-foreground">Provider default: {defaultModel}</span> : null}
                                    <ContextMeter usage={contextUsage} />
                                </>
                            ),
                            controls: (
                                <>
                                    {onStopTurn ? <Button type="button" variant="outline" size="sm" onClick={onStopTurn} data-testid="project-chat-stop">Stop</Button> : null}
                                    {modelSettingsSource === 'conversation' && onUseModelDefaults ? (
                                        <Button type="button" size="sm" variant="ghost" disabled={controlsDisabled} onClick={onUseModelDefaults}>Use defaults</Button>
                                    ) : null}
                                    <ModelChooser inherited={inheritedModelSettings} value={modelSettings} onChange={onModelSettingsChange}
                                        projectPath={activeProjectPath} inheritLabel="Provider default"
                                        layout="compact" disabled={controlsDisabled || isModelSettingsLoading} />
                                </>
                            ),
                        }}
                    >
                        {historyContent}
                    </ChatPane>
                )}
            </div>
        </div>
    )
}
