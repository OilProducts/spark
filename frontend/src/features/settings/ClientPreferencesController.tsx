import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { AppNotice } from '@/components/app/app-notice'
import { Button } from '@/components/ui/button'
import { applyTheme } from '@/lib/theme'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from './services/clientPreferences'

export function ClientPreferencesController() {
    const saved = useRef<ClientPreferencesView | null>(null)
    const draft = useRef<Partial<ClientPreferences> | null>(null)
    const pending = useRef(false)
    // A failed load leaves the defaults in place; a failed save keeps the change to try again.
    const [error, setError] = useState<{ kind: 'load' | 'save'; message: string } | null>(null)
    useEffect(() => {
        let cancelled = false
        const apply = (value: ClientPreferencesView) => {
            saved.current = value
            applyTheme(value.effective.appearance ?? 'system')
            useStore.setState(() => ({
                clientPreferencesLoaded: true,
                clientFlowNodePositions: value.effective.flow_node_positions ?? {},
                clientFlowEdgePorts: value.effective.flow_edge_ports ?? {},
                clientRunPresentation: value.effective.run_presentation ?? {},
                preferredEditorMode: value.effective.editor_mode,
                editorSidebarWidth: value.effective.editor_sidebar_width,
                preferredAdvancedControls: value.effective.show_advanced_controls ?? false,
                preferredExpandChildFlows: value.effective.expand_child_flows ?? false,
                preferredGraphSettingsOpen: value.effective.graph_settings_open ?? false,
                preferredShowThinking: value.effective.show_thinking ?? true,
                preferredHomeSidebarPrimarySplitRatio: value.effective.home_sidebar_primary_split_ratio ?? null,
            }))
        }
        const refresh = () => {
            if (pending.current || draft.current) return
            const base = saved.current
            void fetchClientPreferences().then((value) => {
                if (!cancelled && saved.current === base && !pending.current && !draft.current) { apply(value); setError(null) }
            }).catch(() => { if (!cancelled && saved.current === base && !pending.current && !draft.current) setError({ kind: 'load', message: 'Couldn’t load your preferences, so Spark is using its defaults.' }) })
        }
        const save = async () => {
            if (!draft.current || pending.current || cancelled) return
            const submitted = draft.current
            pending.current = true
            try {
                const base = saved.current ?? await fetchClientPreferences()
                saved.current = base
                const value = await saveClientPreferences(base, { ...base.stored, ...submitted })
                saved.current = value
                useStore.setState({ clientPreferencesLoaded: true })
                if (draft.current === submitted) { draft.current = null; if (!cancelled) setError(null) }
            } catch (error) {
                if (!cancelled) setError({ kind: 'save', message: error instanceof Error ? error.message : 'Unable to save preferences. Your changes are retained.' })
            } finally { pending.current = false }
            // A second completed interaction may arrive while the first save is pending.
            if (draft.current && draft.current !== submitted) void save()
        }
        const complete = (event: Event) => {
            draft.current = { ...(draft.current ?? {}), ...(event as CustomEvent<Partial<ClientPreferences>>).detail }
            void save()
        }
        const retry = () => { if (draft.current) void save(); else refresh() }
        const discard = () => { if (!pending.current) { draft.current = null; refresh() } }
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
