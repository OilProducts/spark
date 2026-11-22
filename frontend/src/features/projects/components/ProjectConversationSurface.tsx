import { useEffect, useRef, useState, type ReactNode } from 'react'
import { DropdownMenu } from 'radix-ui'
import { Check, ChevronDown, Hash, ListTodo, MessageSquare, Square } from 'lucide-react'
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
import type { ConversationChatMode } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { CHAT_HEADING_ID } from '@/app/useAttention'
import { cn } from '@/lib/utils'
import { formatConversationAgeShort, formatConversationTimestamp } from '../model/projectsHomeState'
import { isModelComposerCommand } from '../hooks/useConversationComposer'
import type { useScrollFollow } from '../hooks/useScrollFollow'
import type { ComposerAttachment } from '../model/types'
import { ChatPane } from './ChatPane'
import { ProjectPicker } from './ProjectPicker'
import { ContextMeter, type ContextUsage } from './ContextMeter'

const modeMenuItem = 'relative flex cursor-default select-none items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50'

interface ProjectConversationSurfaceProps {
    activeProjectLabel: string | null
    activeProjectPath: string | null
    /** A chat is open; otherwise the first message starts one in the project. */
    hasOpenChat?: boolean
    activeConversationTitle?: string | null
    activeConversationHandle?: string | null
    activeConversationUpdatedAt?: string | null
    activeChatMode: ConversationChatMode
    /** False when the active harness reports `plan_mode` unsupported; hides Plan unless the chat is already in it. */
    planModeSupported?: boolean
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
    /** Locks the model while the chat loads or a turn is starting; during a turn, edits apply from the next one. */
    isModelSettingsLocked?: boolean
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
    modelPickerOpen?: boolean
    onModelPickerOpenChange?: (open: boolean) => void
}

// Where the chat's model comes from, named in the model button's tooltip/label, and where to change it.
const MODEL_SOURCES = {
    conversation: { prefix: 'Default', title: 'Chosen for this chat with the model picker.' },
    role: { prefix: 'Role default', title: "Set by this chat's role. Pick a model to use another one in this chat." },
    project: { prefix: 'Project default', title: "Change it in this project's settings, or pick a model for this chat." },
    workspace: { prefix: 'Workspace default', title: 'Change it in Settings, or pick a model for this chat.' },
} as const

/** The chat sheet: header, scrolling transcript and the composer. ViewLayout supplies the sheet itself. */
// The chat's ID is how flows and the spark command's --conversation option reach it: shown in the title line and copied with a click.
function CopyChatId({ handle }: { handle: string }) {
    const [status, setStatus] = useState<'idle' | 'copied' | 'failed'>('idle')
    const reset = useRef<number | null>(null)
    useEffect(() => () => { if (reset.current !== null) window.clearTimeout(reset.current) }, [])
    const show = (next: 'copied' | 'failed') => {
        setStatus(next)
        if (reset.current !== null) window.clearTimeout(reset.current)
        reset.current = window.setTimeout(() => setStatus('idle'), next === 'failed' ? 4000 : 1500)
    }
    return (
        <>
            <button
                type="button"
                data-testid="chat-handle"
                data-handle={handle}
                aria-label={status === 'copied' ? 'Chat ID copied' : `Copy chat ID ${handle}`}
                title={`Copy this chat’s ID (${handle}). Flows and the spark command’s --conversation option use it to reach this chat.`}
                onClick={() => {
                    // A browser can refuse the clipboard; the ID stays on screen to copy by hand.
                    Promise.resolve()
                        .then(() => navigator.clipboard.writeText(handle))
                        .then(() => show('copied'), () => show('failed'))
                }}
                className="ml-1 inline-flex shrink-0 items-center gap-1 rounded-sm px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
                {status === 'copied' ? <Check aria-hidden="true" className="size-3.5" /> : <Hash aria-hidden="true" className="size-3.5" />}
                <span className="select-all font-mono text-xs">{status === 'copied' ? 'Copied' : handle}</span>
            </button>
            {status === 'failed' ? (
                <span role="status" data-testid="chat-handle-copy-failed" className="shrink-0 text-xs text-destructive">
                    Couldn’t copy: the browser blocked the clipboard
                </span>
            ) : null}
        </>
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
    planModeSupported = true,
    contextUsage = null,
    onChatModeChange,
    hasThinking,
    showThinking,
    onShowThinkingChange,
    defaultModel,
    modelSettings,
    inheritedModelSettings,
    onModelSettingsChange,
    isModelSettingsLocked = false,
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
    modelPickerOpen,
    onModelPickerOpenChange,
}: ProjectConversationSurfaceProps) {
    const controlsDisabled = !activeProjectPath || isChatInputDisabled
    const hasProject = Boolean(activeProjectPath && activeProjectLabel)
    const [uncontrolledModelPickerOpen, setUncontrolledModelPickerOpen] = useState(false)
    const isModelPickerOpen = modelPickerOpen ?? uncontrolledModelPickerOpen
    const setModelPickerOpen = (next: boolean) => {
        if (modelPickerOpen === undefined) setUncontrolledModelPickerOpen(next)
        onModelPickerOpenChange?.(next)
    }
    const modelSourceTitle = modelSettingsSource ? MODEL_SOURCES[modelSettingsSource].title : undefined
    const modelTriggerTitle = modelSourceTitle
        ? `${modelSourceTitle} (Ctrl+/ or ⌘/)`
        : 'Choose model (Ctrl+/ or ⌘/)'
    return (
        <div
            data-testid="project-ai-conversation-surface"
            onKeyDown={(event) => {
                if (
                    !event.defaultPrevented
                    && (event.metaKey || event.ctrlKey)
                    && !event.altKey
                    && !event.shiftKey
                    && event.key === '/'
                    && activeProjectPath
                    && !isModelSettingsLocked
                ) {
                    event.preventDefault()
                    setModelPickerOpen(true)
                }
            }}
            // A narrow viewport stacks the chat list above; the chat still fills the view's height so it opens at its latest message with the composer in reach.
            className={cn('flex min-h-0 flex-col', isNarrowViewport ? 'h-[100cqh]' : 'h-full')}
        >
            <header className="flex items-center justify-between gap-x-6 border-b border-border px-6 py-4">
                <div className="min-w-0 flex-1">
                    <h1 id={CHAT_HEADING_ID} tabIndex={-1} data-testid="chat-title" title={activeConversationTitle ?? undefined} className="line-clamp-2 break-words rounded-sm text-xl font-semibold text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                        {activeConversationTitle || (hasOpenChat ? 'Chat' : 'New chat')}
                    </h1>
                    {hasProject ? (
                        <p data-testid="chat-project" className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
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
                            sendDisabled: isModelComposerCommand(chatDraft)
                                ? (!activeProjectPath || isModelSettingsLocked)
                                : isChatSendDisabled,
                            allowSubmitWhileUploadBlocking: isModelComposerCommand(chatDraft),
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
                            tone: activeChatMode === 'plan' ? 'accent' : 'default',
                            footer: <ContextMeter usage={contextUsage} />,
                            action: onStopTurn ? (
                                <Button
                                    type="button"
                                    variant="outline"
                                    size="icon-sm"
                                    onClick={onStopTurn}
                                    data-testid="project-chat-stop"
                                    aria-label="Stop"
                                    title="Stop"
                                >
                                    <Square aria-hidden="true" className="size-3.5 fill-current" />
                                </Button>
                            ) : undefined,
                            controls: (
                                <>
                                    {(planModeSupported || activeChatMode === 'plan') ? (
                                        <DropdownMenu.Root>
                                            <DropdownMenu.Trigger asChild>
                                                <Button
                                                    type="button"
                                                    variant="ghost"
                                                    size="sm"
                                                    data-testid="project-active-chat-mode-badge"
                                                    data-mode={activeChatMode}
                                                    aria-label={`Mode: ${activeChatMode === 'plan' ? 'Plan' : 'Chat'}`}
                                                    // Same guard as the /plan command: no switching mid-turn or before the model list is ready.
                                                    disabled={controlsDisabled || isChatSendDisabled || !onChatModeChange}
                                                    className={cn(
                                                        'h-7 px-2 text-xs font-medium has-[>svg]:px-2',
                                                        activeChatMode === 'plan'
                                                            ? 'bg-accent text-accent-foreground hover:bg-accent/80'
                                                            : 'text-foreground hover:bg-muted',
                                                    )}
                                                >
                                                    {activeChatMode === 'plan'
                                                        ? <ListTodo aria-hidden="true" className="size-3.5" />
                                                        : <MessageSquare aria-hidden="true" className="size-3.5" />}
                                                    <span>{activeChatMode === 'plan' ? 'Plan' : 'Chat'}</span>
                                                    <ChevronDown aria-hidden="true" className="size-3.5 text-muted-foreground opacity-60" />
                                                </Button>
                                            </DropdownMenu.Trigger>
                                            <DropdownMenu.Portal>
                                                <DropdownMenu.Content
                                                    side="top"
                                                    align="start"
                                                    sideOffset={6}
                                                    collisionPadding={8}
                                                    aria-label="Mode"
                                                    aria-labelledby={undefined}
                                                    className="z-50 w-64 max-w-[calc(100vw-1rem)] rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md"
                                                >
                                                    <DropdownMenu.Label className="px-2 pb-1 pt-1.5 text-xs font-semibold text-muted-foreground">
                                                        Mode
                                                    </DropdownMenu.Label>
                                                    <DropdownMenu.RadioGroup
                                                        value={activeChatMode}
                                                        onValueChange={(next) => {
                                                            const nextMode: ConversationChatMode = next === 'plan' ? 'plan' : 'chat'
                                                            if (nextMode !== activeChatMode) {
                                                                onChatModeChange?.(nextMode)
                                                            }
                                                        }}
                                                    >
                                                        <DropdownMenu.RadioItem
                                                            value="chat"
                                                            aria-label="Chat"
                                                            aria-description="Discuss and make changes directly"
                                                            className={modeMenuItem}
                                                        >
                                                            <span className="flex min-w-0 flex-1 flex-col">
                                                                <span className="font-medium text-foreground">Chat</span>
                                                                <span className="text-xs text-muted-foreground">Discuss and make changes directly</span>
                                                            </span>
                                                            <DropdownMenu.ItemIndicator className="shrink-0 text-foreground">
                                                                <Check aria-hidden="true" className="size-3.5" />
                                                            </DropdownMenu.ItemIndicator>
                                                        </DropdownMenu.RadioItem>
                                                        <DropdownMenu.RadioItem
                                                            value="plan"
                                                            disabled={!planModeSupported}
                                                            aria-label={planModeSupported ? 'Plan' : 'Plan (unsupported)'}
                                                            aria-description={planModeSupported ? 'Read-only planning before editing' : 'Not supported by this harness'}
                                                            className={modeMenuItem}
                                                        >
                                                            <span className="flex min-w-0 flex-1 flex-col">
                                                                <span className="font-medium text-foreground">{planModeSupported ? 'Plan' : 'Plan (unsupported)'}</span>
                                                                <span className="text-xs text-muted-foreground">
                                                                    {planModeSupported ? 'Read-only planning before editing' : 'Not supported by this harness'}
                                                                </span>
                                                            </span>
                                                            <DropdownMenu.ItemIndicator className="shrink-0 text-foreground">
                                                                <Check aria-hidden="true" className="size-3.5" />
                                                            </DropdownMenu.ItemIndicator>
                                                        </DropdownMenu.RadioItem>
                                                    </DropdownMenu.RadioGroup>
                                                </DropdownMenu.Content>
                                            </DropdownMenu.Portal>
                                        </DropdownMenu.Root>
                                    ) : null}
                                    {/* Compact model + effort controls; tooltips and aria-labels name the inheritance source. */}
                                    <ModelChooser inherited={inheritedModelSettings} value={modelSettings} onChange={onModelSettingsChange}
                                        projectPath={activeProjectPath} inheritLabel={defaultModel || 'Provider default'}
                                        defaultPrefix={modelSettingsSource ? MODEL_SOURCES[modelSettingsSource].prefix : undefined}
                                        title={modelTriggerTitle}
                                        open={isModelPickerOpen} onOpenChange={setModelPickerOpen}
                                        layout="compact" disabled={!activeProjectPath || isModelSettingsLocked} />
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
