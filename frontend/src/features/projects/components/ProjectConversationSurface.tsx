import { useRef, useState, type ClipboardEvent, type DragEvent, type FormEvent, type KeyboardEvent, type ReactNode, type RefObject } from 'react'
import { ArrowUp, Folder, Paperclip } from 'lucide-react'
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
import { Textarea } from '@/components/ui/textarea'
import type { ConversationChatMode } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { cn } from '@/lib/utils'
import { formatConversationAgeShort, formatConversationTimestamp } from '../model/projectsHomeState'
import type { ComposerAttachment } from '../model/types'
import { ComposerAttachments } from './ComposerAttachments'
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
    isConversationPinnedToBottom: boolean
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

const MODEL_SOURCE_LABELS = {
    conversation: 'Conversation override',
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
    isConversationPinnedToBottom,
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
    const hasProject = Boolean(activeProjectPath && activeProjectLabel)
    const fileInputRef = useRef<HTMLInputElement>(null)
    const [isDraggingFiles, setIsDraggingFiles] = useState(false)
    const canAttach = Boolean(onAddAttachments) && !controlsDisabled
    const hasMessage = chatDraft.trim().length > 0 || attachments.length > 0
    // Clipboard files are attached; any plain text alongside them still
    // pastes into the message box through the default paste.
    const onComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
        const files = Array.from(event.clipboardData.files)
        if (!canAttach || files.length === 0) return
        if (!Array.from(event.clipboardData.types).includes('text/plain')) event.preventDefault()
        onAddAttachments?.(files)
    }
    const dragHasFiles = (event: DragEvent) => Array.from(event.dataTransfer.types).includes('Files')
    const onComposerDragOver = (event: DragEvent<HTMLFormElement>) => {
        if (!canAttach || !dragHasFiles(event)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'copy'
        setIsDraggingFiles(true)
    }
    const onComposerDrop = (event: DragEvent<HTMLFormElement>) => {
        setIsDraggingFiles(false)
        if (!canAttach || !dragHasFiles(event)) return
        event.preventDefault()
        onAddAttachments?.(Array.from(event.dataTransfer.files))
    }
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
                    <>
                        <div
                            ref={conversationBodyRef}
                            data-testid="project-ai-conversation-body"
                            onScroll={onSyncConversationPinnedState}
                            className={cn('px-6 pb-3 pt-6', !isNarrowViewport && 'min-h-0 flex-1 overflow-y-auto')}
                        >
                            <div className="mx-auto w-full max-w-3xl">{historyContent}</div>
                        </div>
                        <div className="shrink-0 px-6 pb-5 pt-2">
                            <div className="mx-auto flex w-full max-w-3xl flex-col gap-2">
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
                                {chatModelAvailabilityMessage ? (
                                    <InlineError data-testid="project-chat-model-availability" dense>{chatModelAvailabilityMessage}</InlineError>
                                ) : null}
                                <form
                                    data-testid="project-ai-conversation-composer"
                                    onSubmit={onChatComposerSubmit}
                                    onDragOver={onComposerDragOver}
                                    onDragLeave={() => setIsDraggingFiles(false)}
                                    onDrop={onComposerDrop}
                                    className={cn(
                                        'flex flex-col rounded-xl border border-input bg-background transition-[color,box-shadow] focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30',
                                        isDraggingFiles && 'border-ring ring-2 ring-ring/30',
                                    )}
                                >
                                    <ComposerAttachments attachments={attachments} onRetry={onRetryAttachment} onRemove={onRemoveAttachment} isSending={isAttachmentSendPending} />
                                    <Textarea
                                        id="project-ai-conversation-input"
                                        data-testid="project-ai-conversation-input"
                                        value={chatDraft}
                                        onChange={(event) => onChatDraftChange(event.target.value)}
                                        onKeyDown={onChatComposerKeyDown}
                                        onPaste={onComposerPaste}
                                        aria-label="Message"
                                        placeholder="Reply, or describe a change to work on"
                                        className="max-h-[40vh] min-h-0 resize-none overflow-y-auto rounded-xl border-0 bg-transparent px-3.5 pb-1 pt-3 focus-visible:border-0 focus-visible:ring-0"
                                        rows={2}
                                    />
                                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2 pl-3.5 pr-2">
                                        <p data-testid="chat-composer-project" className="inline-flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                                            <Folder aria-hidden="true" className="size-3.5 shrink-0" />
                                            <span className="truncate">Runs and missions started here go to <span className="text-foreground">{activeProjectLabel}</span></span>
                                        </p>
                                        {modelSettingsSource ? <span className="text-xs text-muted-foreground">{MODEL_SOURCE_LABELS[modelSettingsSource]}</span> : null}
                                        {defaultModel ? <span className="text-xs text-muted-foreground">Provider default: {defaultModel}</span> : null}
                                        <ContextMeter usage={contextUsage} />
                                        <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
                                            {onAddAttachments ? (
                                                <>
                                                    <input
                                                        ref={fileInputRef}
                                                        data-testid="project-ai-conversation-file-input"
                                                        type="file"
                                                        multiple
                                                        hidden
                                                        onChange={(event) => {
                                                            onAddAttachments(Array.from(event.target.files ?? []))
                                                            event.target.value = ''
                                                        }}
                                                    />
                                                    <Button
                                                        type="button"
                                                        data-testid="project-ai-conversation-attach-button"
                                                        variant="ghost"
                                                        size="icon-sm"
                                                        aria-label="Attach files"
                                                        title="Attach files (up to 10, 20 MB each, 50 MB per message)"
                                                        disabled={!canAttach}
                                                        onClick={() => fileInputRef.current?.click()}
                                                    >
                                                        <Paperclip />
                                                    </Button>
                                                </>
                                            ) : null}
                                            {onStopTurn ? <Button type="button" variant="outline" size="sm" onClick={onStopTurn} data-testid="project-chat-stop">Stop</Button> : null}
                                            {modelSettingsSource === 'conversation' && onUseModelDefaults ? (
                                                <Button type="button" size="sm" variant="ghost" disabled={controlsDisabled} onClick={onUseModelDefaults}>Use defaults</Button>
                                            ) : null}
                                            <ModelChooser inherited={inheritedModelSettings} value={modelSettings} onChange={onModelSettingsChange}
                                                projectPath={activeProjectPath} inheritLabel="Provider default"
                                                layout="compact" disabled={controlsDisabled || isModelSettingsLoading} />
                                            <Button
                                                data-testid="project-ai-conversation-send-button"
                                                type="submit"
                                                size="icon-sm"
                                                aria-label={chatSendButtonLabel}
                                                title={chatSendButtonLabel}
                                                disabled={!hasMessage || isUploadBlocking || isChatSendDisabled}
                                            >
                                                <ArrowUp />
                                            </Button>
                                        </div>
                                    </div>
                                </form>
                            </div>
                        </div>
                    </>
                )}
            </div>
        </div>
    )
}
