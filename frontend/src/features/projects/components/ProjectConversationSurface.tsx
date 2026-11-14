import { useState, type ReactNode } from 'react'
import { Check, ChevronDown, Folder, Hash } from 'lucide-react'
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
import { ProjectPicker } from './ProjectPicker'
import { ContextMeter, type ContextUsage } from './ContextMeter'

interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    /** A chat is open; otherwise the first message starts one in the project. */
    hasOpenChat?: boolean
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

// Where the chat's model comes from, named on the model button, and where to change it.
const MODEL_SOURCES = {
    conversation: { prefix: 'Default', title: 'Chosen for this chat with the model picker.' },
    role: { prefix: 'Role default', title: "Set by this chat's role. Pick a model to use another one in this chat." },
    project: { prefix: 'Project default', title: "Change it in this project's settings, or pick a model for this chat." },
    workspace: { prefix: 'Default', title: 'Change it in Settings, or pick a model for this chat.' },
} as const

/** The chat sheet: header, scrolling transcript and the composer. ViewLayout supplies the sheet itself. */
// The chat's ID is how flows and the spark command's --conversation option reach it: kept a click away, not in the title line.
function CopyChatId({ handle }: { handle: string }) {
    const [copied, setCopied] = useState(false)
    return (
        <button
            type="button"
            data-testid="chat-handle"
            data-handle={handle}
            aria-label={copied ? 'Chat ID copied' : `Copy chat ID ${handle}`}
            title={`Copy this chat’s ID (${handle}). Flows and the spark command’s --conversation option use it to reach this chat.`}
            onClick={() => {
                void navigator.clipboard?.writeText(handle).then(() => {
                    setCopied(true)
                    window.setTimeout(() => setCopied(false), 1500)
                })
            }}
            className="ml-1 inline-flex shrink-0 items-center gap-1 rounded-sm px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
            {copied ? <Check aria-hidden="true" className="size-3.5" /> : <Hash aria-hidden="true" className="size-3.5" />}
            <span className="text-xs">{copied ? 'Copied' : 'Copy ID'}</span>
        </button>
    )
}

export function ProjectConversationSurface({
    activeProjectLabel,
    activeProjectPath,
    hasOpenChat = true,
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
            // A narrow viewport stacks the chat list above; the chat still fills one screen so it opens at its latest message with the composer in reach.
            className={cn('flex min-h-0 flex-col', isNarrowViewport ? 'h-[calc(100dvh-3.25rem)]' : 'h-full')}
        >
            <header className="flex items-center justify-between gap-x-6 border-b border-border px-6 py-4">
                <div className="min-w-0 flex-1">
                    <h1 data-testid="chat-title" title={activeConversationTitle ?? undefined} className="line-clamp-2 break-words text-xl font-semibold text-foreground">
                        {activeConversationTitle || (hasOpenChat ? 'Chat' : 'New chat')}
                    </h1>
                    {hasProject ? (
                        <p className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
                            {hasOpenChat ? (
                                <button
                                    type="button"
                                    data-testid="chat-project-link"
                                    title={activeProjectPath ?? undefined}
                                    onClick={() => activeProjectPath && useStore.getState().openProjectPage(activeProjectPath)}
                                    className="min-w-0 truncate outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                                >
                                    {activeProjectLabel}
                                </button>
                            ) : (
                                // Nothing is sent yet, so the chat can still start in another project.
                                <ProjectPicker
                                    heading="Start this chat in"
                                    testId="new-chat-project-picker"
                                    defaultProjectPath={activeProjectPath}
                                    onPick={(projectPath) => window.dispatchEvent(new CustomEvent('spark:new-chat', { detail: projectPath }))}
                                >
                                    <button
                                        type="button"
                                        data-testid="new-chat-project-button"
                                        title={activeProjectPath ?? undefined}
                                        aria-label={`New chat in ${activeProjectLabel}; change project`}
                                        className="flex min-w-0 items-center gap-1 rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                                    >
                                        <span className="shrink-0">in</span>
                                        <span className="min-w-0 truncate font-medium text-foreground">{activeProjectLabel}</span>
                                        <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
                                    </button>
                                </ProjectPicker>
                            )}
                            {activeConversationUpdatedAt ? (
                                <>
                                    <span aria-hidden="true">·</span>
                                    <span className="shrink-0" title={formatConversationTimestamp(activeConversationUpdatedAt)}>
                                        {formatConversationAgeShort(activeConversationUpdatedAt)}
                                    </span>
                                </>
                            ) : null}
                            {activeConversationHandle ? <CopyChatId handle={activeConversationHandle} /> : null}
                        </p>
                    ) : null}
                </div>
                {hasThinking ? <ShowThinkingToggle pressed={showThinking} onPressedChange={onShowThinkingChange} className="shrink-0" /> : null}
            </header>
            <div className="flex min-h-0 flex-1 flex-col">
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
                        scrolls
                        hasContent={hasRenderableConversationHistory}
                        notice={chatModelAvailabilityMessage ? (
                            <InlineError data-testid="project-chat-model-availability" dense>{chatModelAvailabilityMessage}</InlineError>
                        ) : null}
                        composer={{
                            placeholder: hasOpenChat ? 'Reply, or describe a change to work on' : `Start a new chat in ${activeProjectLabel}`,
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
                                    <ContextMeter usage={contextUsage} />
                                </>
                            ),
                            controls: (
                                <>
                                    {onStopTurn ? <Button type="button" variant="outline" size="sm" onClick={onStopTurn} data-testid="project-chat-stop">Stop</Button> : null}
                                    {modelSettingsSource === 'conversation' && onUseModelDefaults ? (
                                        <Button type="button" size="sm" variant="ghost" disabled={controlsDisabled} onClick={onUseModelDefaults}>Use defaults</Button>
                                    ) : null}
                                    {activeChatMode ? (
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
                                            <NativeSelectOption value="chat">Chat mode</NativeSelectOption>
                                            <NativeSelectOption value="plan">Plan mode</NativeSelectOption>
                                        </NativeSelect>
                                    ) : null}
                                    {/* One label for the model: the button names it and, when inherited, where from. */}
                                    <ModelChooser inherited={inheritedModelSettings} value={modelSettings} onChange={onModelSettingsChange}
                                        projectPath={activeProjectPath} inheritLabel={defaultModel || 'Provider default'}
                                        defaultPrefix={modelSettingsSource ? MODEL_SOURCES[modelSettingsSource].prefix : undefined}
                                        title={modelSettingsSource ? MODEL_SOURCES[modelSettingsSource].title : undefined}
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
