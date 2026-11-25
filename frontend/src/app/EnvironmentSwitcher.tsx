import { useEffect } from 'react'
import { AlertTriangle, Laptop, Loader2, RefreshCw, Server } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useLiveThinkingStore } from '@/components/app/transcript/liveThinking'
import { resetModelOptionsCache } from '@/components/model-chooser/useModelOptions'
import { useRunJournalStore } from '@/features/runs/state/runJournalStore'
import { useRunTranscriptStore } from '@/features/runs/state/runTranscriptStore'
import {
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
    const activeLink = activeEnvironmentId
        ? links.find((link) => link.environment_id === activeEnvironmentId) ?? null
        : null

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
            {activeEnvironmentId !== null && status === 'ready' ? (
                <div
                    data-testid="remote-environment-indicator"
                    className="hidden shrink-0 items-center gap-2 text-xs text-muted-foreground sm:flex"
                >
                    <span className="inline-block size-1.5 rounded-full bg-primary" aria-hidden="true" />
                    <span className="max-w-56 truncate" title={activeLink?.origin ?? remoteDescriptor?.display_name}>
                        Remote{activeLink?.origin ? ` · ${activeLink.origin}` : ''}
                    </span>
                    {remoteDescriptor && remoteDescriptor.ui_api_version > 0 ? (
                        <span>· UI API v{remoteDescriptor.ui_api_version}</span>
                    ) : null}
                </div>
            ) : null}
        </div>
    )
}

/**
 * Shown in place of the workspace while checking a linked remote environment or when its
 * `ui_api_version` / `protocol_version` does not match this Spark instance.
 */
export function RemoteEnvironmentGate() {
    const { selectedEnvironmentId, status, error, links, localDescriptor, remoteDescriptor } = useActiveEnvironment()
    const link = selectedEnvironmentId
        ? links.find((entry) => entry.environment_id === selectedEnvironmentId) ?? null
        : null
    const name = link?.label || selectedEnvironmentId || 'Remote environment'

    return (
        <main
            id="app-main"
            tabIndex={-1}
            data-testid="remote-environment-gate"
            className="flex flex-1 min-h-0 flex-col items-center justify-center p-6 text-center outline-none"
        >
            <div className="w-full max-w-md space-y-4 rounded-xl border border-border bg-background p-6 shadow-sm">
                {status === 'checking' ? (
                    <>
                        <div className="flex items-center justify-center gap-2 text-sm font-semibold text-foreground">
                            <Loader2 aria-hidden="true" className="size-4 animate-spin text-muted-foreground" />
                            <span>Connecting to {name}…</span>
                        </div>
                        <p className="text-sm text-muted-foreground">
                            Checking Spark UI compatibility before opening this environment.
                        </p>
                        <div className="flex justify-center gap-2">
                            <Button type="button" size="sm" variant="outline" onClick={() => void selectEnvironment(null)}>
                                Return to this machine
                            </Button>
                        </div>
                    </>
                ) : (
                    <>
                        <div className="flex items-center justify-center gap-2 text-sm font-semibold text-foreground">
                            <AlertTriangle aria-hidden="true" className="size-4 text-warning" />
                            <span>
                                {status === 'incompatible'
                                    ? `Version mismatch with ${name}`
                                    : `Couldn’t connect to ${name}`}
                            </span>
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
                                    <dd className="text-muted-foreground">
                                        UI API v{localDescriptor.ui_api_version} · protocol v{localDescriptor.protocol_version}
                                        {localDescriptor.spark_version ? ` · ${localDescriptor.spark_version}` : ''}
                                    </dd>
                                </div>
                                <div>
                                    <dt className="font-medium text-foreground">{name}</dt>
                                    <dd className="text-muted-foreground">
                                        {remoteDescriptor.ui_api_version > 0
                                            ? `UI API v${remoteDescriptor.ui_api_version}`
                                            : 'Unversioned UI API'}
                                        {' · '}protocol v{remoteDescriptor.protocol_version}
                                        {remoteDescriptor.spark_version ? ` · ${remoteDescriptor.spark_version}` : ''}
                                    </dd>
                                </div>
                            </dl>
                        ) : null}
                        <div className="flex flex-wrap justify-center gap-2">
                            <Button type="button" size="sm" variant="outline" onClick={() => void retrySelectedEnvironment()}>
                                <RefreshCw aria-hidden="true" className="size-3.5" />
                                Retry
                            </Button>
                            <Button type="button" size="sm" onClick={() => void selectEnvironment(null)}>
                                Return to this machine
                            </Button>
                        </div>
                    </>
                )}
            </div>
        </main>
    )
}
