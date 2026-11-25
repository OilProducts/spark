import { useEffect } from 'react'
import { AlertTriangle, Laptop, Loader2, RefreshCw, Server } from 'lucide-react'
import { Dialog as DialogPrimitive } from 'radix-ui'
import { Button } from '@/components/ui/button'
import { useLiveThinkingStore } from '@/components/app/transcript/liveThinking'
import { resetModelOptionsCache } from '@/components/model-chooser/useModelOptions'
import { useRunJournalStore } from '@/features/runs/state/runJournalStore'
import { useRunTranscriptStore } from '@/features/runs/state/runTranscriptStore'
import {
    describeEnvironmentVersion,
    ENVIRONMENT_LINKS_CHANGED_EVENT,
    refreshLocalEnvironments,
    registerEnvironmentResetHandler,
    retrySelectedEnvironment,
    selectEnvironment,
    useActiveEnvironment,
} from '@/lib/activeEnvironment'
import { resetLlmProfilesCache } from '@/lib/useLlmProfiles'
import { cn } from '@/lib/utils'
import { resetStoreForEnvironmentSwitch } from '@/store'
import { resetAttentionState } from './useAttention'

let resetHandlerRegistered = false

function ensureEnvironmentResetHandler() {
    if (resetHandlerRegistered) return
    resetHandlerRegistered = true
    registerEnvironmentResetHandler((targetEnvironmentId) => {
        resetStoreForEnvironmentSwitch(targetEnvironmentId === null)
        useRunJournalStore.setState({ byRunId: {} })
        useRunTranscriptStore.setState({ byRunId: {} })
        useLiveThinkingStore.getState().clear()
        resetAttentionState()
        resetLlmProfilesCache()
        resetModelOptionsCache()
    })
}

export function EnvironmentBootstrap() {
    useEffect(() => {
        ensureEnvironmentResetHandler()
        const refresh = () => {
            void refreshLocalEnvironments()
        }
        refresh()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, refresh)
        window.addEventListener('focus', refresh)
        return () => {
            window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [])
    return null
}

/**
 * Top environment switcher bar. Hidden for single-machine users with no linked environments so
 * there is zero extra chrome until at least one remote Spark environment is linked.
 */
export function EnvironmentSwitcherBar() {
    const {
        activeEnvironmentId,
        selectedEnvironmentId,
        status,
        localDescriptor,
        remoteDescriptor,
        links,
    } = useActiveEnvironment()

    if (links.length === 0 && selectedEnvironmentId === null) {
        return null
    }

    const localLabel = localDescriptor?.display_name || 'This machine'
    const selectedMissingLink =
        selectedEnvironmentId !== null
        && !links.some((link) => link.environment_id === selectedEnvironmentId)
    const activeLink = links.find((link) => link.environment_id === activeEnvironmentId)
    const selectedLink = links.find((link) => link.environment_id === selectedEnvironmentId)

    return (
        <div
            data-testid="environment-switcher"
            className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-border bg-ground px-2 text-xs"
        >
            <div
                role="tablist"
                aria-label="Spark environments"
                className="flex min-w-0 items-center gap-1 overflow-x-auto"
            >
                <button
                    type="button"
                    role="tab"
                    data-testid="environment-tab-local"
                    aria-selected={selectedEnvironmentId === null}
                    onClick={() => void selectEnvironment(null)}
                    className={cn(
                        'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2.5 font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
                        selectedEnvironmentId === null
                            ? 'border-border bg-background text-foreground'
                            : 'border-transparent text-muted-foreground hover:bg-background/60 hover:text-foreground',
                    )}
                >
                    <Laptop aria-hidden="true" className="size-3.5 shrink-0" />
                    <span>{localLabel}</span>
                    {localDescriptor?.display_name ? (
                        <span className="text-xs font-normal text-muted-foreground">(local)</span>
                    ) : null}
                </button>
                {links.map((link) => {
                    const selected = selectedEnvironmentId === link.environment_id
                    const label = link.label || link.environment_id
                    return (
                        <button
                            key={link.environment_id}
                            type="button"
                            role="tab"
                            data-testid={`environment-tab-${link.environment_id}`}
                            aria-selected={selected}
                            title={`${label} (${link.origin})`}
                            onClick={() => void selectEnvironment(link.environment_id)}
                            className={cn(
                                'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2.5 font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
                                selected
                                    ? 'border-border bg-background text-foreground'
                                    : 'border-transparent text-muted-foreground hover:bg-background/60 hover:text-foreground',
                            )}
                        >
                            <Server aria-hidden="true" className="size-3.5 shrink-0" />
                            <span>{label}</span>
                        </button>
                    )
                })}
                {selectedMissingLink ? (
                    <button
                        type="button"
                        role="tab"
                        aria-selected
                        className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-border bg-background px-2.5 font-medium text-foreground"
                    >
                        <Server aria-hidden="true" className="size-3.5 shrink-0" />
                        <span>{selectedEnvironmentId}</span>
                    </button>
                ) : null}
            </div>
            {status === 'checking' ? (
                <div role="status" data-testid="environment-checking" className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
                    <Loader2 aria-hidden="true" className="size-3.5 animate-spin" />
                    <span className="max-w-56 truncate">Connecting to {selectedLink?.label || selectedEnvironmentId}…</span>
                </div>
            ) : activeEnvironmentId !== null && status === 'ready' && remoteDescriptor ? (
                <div
                    data-testid="remote-environment-indicator"
                    title={`${remoteDescriptor.display_name} · ${describeEnvironmentVersion(remoteDescriptor)}`}
                    className="hidden shrink-0 items-center gap-2 text-xs text-muted-foreground sm:flex"
                >
                    <span className="inline-block size-1.5 rounded-full bg-primary" aria-hidden="true" />
                    <span className="max-w-56 truncate">Remote{activeLink ? ` · ${activeLink.origin}` : ''}</span>
                    <span>· UI API v{remoteDescriptor.ui_api_version}</span>
                </div>
            ) : null}
        </div>
    )
}

/**
 * Shown, with the workspace hidden but still mounted, when a linked environment fails its check: unreachable, a
 * rejected credential or another identity, or a `ui_api_version` / `protocol_version` this bundle does not speak.
 * A mounted one that failed stays behind it while another is checked or awaits approval, which it shows. It is a modal layer, with the
 * switcher bar, over any dialog the workspace left open, which waits under it; the dialog provider holds the ones asked
 * meanwhile behind it, and brings a question about leaving, such as a switch's confirmation, over it.
 */
export function RemoteEnvironmentGate() {
    const { activeEnvironmentId, selectedEnvironmentId, status, error, links, localDescriptor, remoteDescriptor } = useActiveEnvironment()
    // The mounted environment failed, and the one chosen is still checked, or passed and awaits approval.
    const switching = status === 'checking' || status === 'ready'
    const nameOf = (environmentId: string) => links.find((entry) => entry.environment_id === environmentId)?.label || environmentId
    const name = selectedEnvironmentId ? nameOf(selectedEnvironmentId) : 'this machine'
    // Back to the workspace still mounted, or to this machine when that is the one failing.
    const returnTo = selectedEnvironmentId === activeEnvironmentId ? null : activeEnvironmentId

    return (
        <DialogPrimitive.Root open>
            <DialogPrimitive.Portal>
                <DialogPrimitive.Content
                    aria-describedby={undefined}
                    // Keys pressed here stay here: shortcuts of the hidden workspace, such as the flow editor's undo, listen on the document.
                    onKeyDown={(event) => event.stopPropagation()}
                    className="fixed inset-0 z-50 flex flex-col bg-ground text-foreground antialiased outline-none"
                >
                    <EnvironmentSwitcherBar />
                    <main
                        data-testid="remote-environment-gate"
                        className="flex flex-1 min-h-0 flex-col items-center justify-center p-6 text-center outline-none"
                    >
                        <div className="w-full max-w-md space-y-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                            <div className="flex items-center justify-center gap-2 text-sm font-semibold text-foreground">
                                {switching
                                    ? <Loader2 aria-hidden="true" className="size-4 animate-spin" />
                                    : <AlertTriangle aria-hidden="true" className="size-4 text-warning" />}
                                <DialogPrimitive.Title asChild>
                                    <span>
                                        {status === 'checking' ? `Connecting to ${name}…`
                                            : status === 'ready' ? `Switching to ${name}…`
                                            : status === 'incompatible' ? `Version mismatch with ${name}`
                                            : `Couldn’t connect to ${name}`}
                                    </span>
                                </DialogPrimitive.Title>
                            </div>
                            {error ? (
                                <p role="alert" className="text-sm text-muted-foreground">
                                    {error}
                                </p>
                            ) : null}
                            {status === 'incompatible' && localDescriptor && remoteDescriptor ? (
                                <dl className="grid grid-cols-2 gap-2 rounded-lg border border-border bg-ground p-3 text-left text-xs">
                                    <div>
                                        <dt className="font-medium text-foreground">This Spark</dt>
                                        <dd className="text-muted-foreground">{describeEnvironmentVersion(localDescriptor)}</dd>
                                    </div>
                                    <div>
                                        <dt className="font-medium text-foreground">{name}</dt>
                                        <dd className="text-muted-foreground">{describeEnvironmentVersion(remoteDescriptor)}</dd>
                                    </div>
                                </dl>
                            ) : null}
                            <div className="flex flex-wrap justify-center gap-2">
                                {switching ? null : (
                                    <Button type="button" size="sm" variant="outline" onClick={() => void retrySelectedEnvironment()}>
                                        <RefreshCw aria-hidden="true" className="size-3.5" />
                                        Retry
                                    </Button>
                                )}
                                <Button type="button" size="sm" onClick={() => void selectEnvironment(returnTo)}>
                                    Return to {returnTo === null ? 'this machine' : nameOf(returnTo)}
                                </Button>
                            </div>
                        </div>
                    </main>
                </DialogPrimitive.Content>
            </DialogPrimitive.Portal>
        </DialogPrimitive.Root>
    )
}
