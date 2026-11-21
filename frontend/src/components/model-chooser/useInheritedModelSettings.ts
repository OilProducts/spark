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

// Read the parent scope, never the override that the picker will remove.
export function useInheritedModelSettingsView(projectPath?: string | null): InheritedModelSettingsState {
    const [settings, setSettings] = useState<LoadedState>()
    useEffect(() => {
        if (projectPath === null) return
        let cancelled = false
        const refresh = () => {
            void fetchModelSettings(projectPath || undefined).then(
                (view) => {
                    if (!cancelled) {
                        setSettings({
                            path: projectPath,
                            status: 'loaded',
                            effective: view.effective ?? undefined,
                            source: view.source,
                        })
                    }
                },
                () => {
                    if (!cancelled) {
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
