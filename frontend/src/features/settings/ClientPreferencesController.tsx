import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { AppNotice } from '@/components/app/app-notice'
import { Button } from '@/components/ui/button'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from './services/clientPreferences'

// The change each environment's save refused, with why. It outlives the workspace that made it, as a
// switch unmounts it: that environment's workspace shows it again, unsaved, with Try again and Discard,
// until it saves or is discarded.
// ponytail: kept for the page's life; a workspace already back before the refusal arrives shows it once remounted.
const refusedChanges = new Map<string, { draft: Partial<ClientPreferences>; message: string }>()

export function ClientPreferencesController() {
    const saved = useRef<ClientPreferencesView | null>(null)
    const draft = useRef<Partial<ClientPreferences> | null>(null)
    const pending = useRef(false)
    // A failed load leaves the defaults in place; a failed save keeps the change to try again.
    const [error, setError] = useState<{ kind: 'load' | 'save'; message: string } | null>(() => {
        const refused = refusedChanges.get(getActiveEnvironmentId() ?? 'local')
        return refused ? { kind: 'save', message: refused.message } : null
    })
    useEffect(() => {
        let cancelled = false
        // These preferences, their writes included, are the environment this mounted in; once it is
        // left, late answers change nothing.
        const environmentId = getActiveEnvironmentId()
        const scope = environmentId ?? 'local'
        draft.current = refusedChanges.get(scope)?.draft ?? null
        const current = () => !cancelled && getActiveEnvironmentId() === environmentId
        const apply = (value: ClientPreferencesView) => {
            saved.current = value
            // A change not saved yet shows over what is saved; changes only carry set values.
            const effective = { ...value.effective, ...draft.current } as ClientPreferencesView['effective']
            // The appearance is this browser's, kept in it (lib/theme), not an environment's: none replaces it.
            useStore.setState(() => ({
                clientPreferencesLoaded: true,
                clientFlowNodePositions: effective.flow_node_positions ?? {},
                clientFlowEdgePorts: effective.flow_edge_ports ?? {},
                chatProjectOrder: effective.chat_project_order ?? [],
                clientRunPresentation: effective.run_presentation ?? {},
                preferredEditorMode: effective.editor_mode,
                editorSidebarWidth: effective.editor_sidebar_width,
                preferredAdvancedControls: effective.show_advanced_controls ?? false,
                preferredExpandChildFlows: effective.expand_child_flows ?? false,
                preferredGraphSettingsOpen: effective.graph_settings_open ?? false,
                preferredShowThinking: effective.show_thinking ?? true,
                preferredHomeSidebarPrimarySplitRatio: effective.home_sidebar_primary_split_ratio ?? null,
            }))
        }
        // Once loaded, a change not saved yet keeps what shows until it saves or is discarded.
        const refresh = () => {
            if (pending.current || (draft.current && saved.current)) return
            const base = saved.current
            void fetchClientPreferences(environmentId).then((value) => {
                if (current() && saved.current === base && !pending.current) { apply(value); if (!draft.current) setError(null) }
            }).catch(() => { if (current() && saved.current === base && !pending.current && !draft.current) setError({ kind: 'load', message: 'Couldn’t load your preferences, so Spark is using its defaults.' }) })
        }
        // A change completed here is saved here even if the workspace unmounts first, as a switch does
        // while an earlier save is in flight; only what it shows waits on current().
        const save = async () => {
            if (!draft.current || pending.current) return
            const submitted = draft.current
            pending.current = true
            try {
                const base = saved.current ?? await fetchClientPreferences(environmentId)
                saved.current = base
                const value = await saveClientPreferences(base, { ...base.stored, ...submitted }, environmentId)
                saved.current = value
                if (current()) useStore.setState({ clientPreferencesLoaded: true })
                if (draft.current === submitted) { draft.current = null; refusedChanges.delete(scope); if (current()) setError(null) }
            } catch (error) {
                const message = error instanceof Error ? error.message : 'Unable to save preferences. Your changes are retained.'
                refusedChanges.set(scope, { draft: draft.current ?? submitted, message })
                if (current()) setError({ kind: 'save', message })
            } finally { pending.current = false }
            // A second completed interaction may arrive while the first save is pending.
            if (draft.current && draft.current !== submitted) void save()
        }
        const complete = (event: Event) => {
            draft.current = { ...(draft.current ?? {}), ...(event as CustomEvent<Partial<ClientPreferences>>).detail }
            void save()
        }
        const retry = () => { if (draft.current) void save(); else refresh() }
        const discard = () => { if (!pending.current) { draft.current = null; refusedChanges.delete(scope); refresh() } }
        refresh()
        window.addEventListener('spark:preferences-completed', complete)
        window.addEventListener('spark:preferences-retry', retry)
        window.addEventListener('spark:preferences-discard', discard)
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => {
            cancelled = true
            window.removeEventListener('spark:preferences-completed', complete)
            window.removeEventListener('spark:preferences-retry', retry)
            window.removeEventListener('spark:preferences-discard', discard)
            window.removeEventListener('spark:settings-live-event', refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [])
    if (!error) return null
    const retry = () => window.dispatchEvent(new Event('spark:preferences-retry'))
    return (
        <AppNotice
            key={`${error.kind}:${error.message}`}
            data-testid="client-preferences-error"
            title={error.kind === 'save' ? 'Couldn’t save your preferences' : undefined}
            actions={error.kind === 'save' ? (
                <>
                    <Button type="button" variant="outline" size="xs" onClick={retry}>Try again</Button>
                    <Button type="button" variant="ghost" size="xs" onClick={() => window.dispatchEvent(new Event('spark:preferences-discard'))}>Discard changes</Button>
                </>
            ) : <Button type="button" variant="outline" size="xs" onClick={retry}>Try again</Button>}
        >
            {error.message}
        </AppNotice>
    )
}
