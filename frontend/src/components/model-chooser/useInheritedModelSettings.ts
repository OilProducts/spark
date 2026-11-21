import { useEffect, useState } from 'react'
import { fetchModelSettings, type ModelSettings, type ModelSettingsView } from '@/lib/api/settingsApi'

export interface InheritedModelSettingsState {
    status: 'idle' | 'loading' | 'loaded' | 'failed'
    effective: ModelSettings | undefined
    source: ModelSettingsView['source'] | undefined
}

type LoadedState = {
    path: string | null | undefined
    status: 'loaded' | 'failed'
    effective: ModelSettings | undefined
    source: ModelSettingsView['source'] | undefined
}

const MODEL_SETTINGS_SECTIONS = new Set(['models', 'project_models', 'llm_profiles', 'providers'])

// Read the parent scope, never the override that the picker will remove.
export function useInheritedModelSettingsView(projectPath?: string | null): InheritedModelSettingsState {
    const [settings, setSettings] = useState<LoadedState>()
    useEffect(() => {
        if (projectPath === null) return
        let cancelled = false
        let revision = 0
        const refresh = (event?: Event) => {
            if (event?.type === 'spark:settings-live-event') {
                const section = (event as CustomEvent | undefined)?.detail?.payload?.section
                if (typeof section === 'string' && !MODEL_SETTINGS_SECTIONS.has(section)) return
                // Clear stale cached defaults while reloading after a model-settings change.
                if (!section || section === 'models' || section === 'project_models') {
                    setSettings(undefined)
                }
            }
            const request = ++revision
            void fetchModelSettings(projectPath || undefined).then(
                (view) => {
                    if (!cancelled && request === revision) {
                        setSettings({
                            path: projectPath,
                            status: 'loaded',
                            effective: view.effective ?? undefined,
                            source: view.source,
                        })
                    }
                },
                () => {
                    if (!cancelled && request === revision) {
                        setSettings({
                            path: projectPath,
                            status: 'failed',
                            effective: undefined,
                            source: undefined,
                        })
                    }
                },
            )
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => {
            cancelled = true
            window.removeEventListener('spark:settings-live-event', refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [projectPath])

    if (projectPath === null) {
        return { status: 'idle', effective: undefined, source: undefined }
    }
    if (!settings || settings.path !== projectPath) {
        return { status: 'loading', effective: undefined, source: undefined }
    }
    return {
        status: settings.status,
        effective: settings.effective,
        source: settings.source,
    }
}

export function useInheritedModelSettings(projectPath?: string | null): ModelSettings | undefined {
    return useInheritedModelSettingsView(projectPath).effective
}
