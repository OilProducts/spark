import { useEffect, useState } from 'react'
import { fetchModelSettings, type ModelSettings } from '@/lib/api/settingsApi'

// Read the parent scope, never the override that the picker will remove.
export function useInheritedModelSettings(projectPath?: string | null) {
    const [settings, setSettings] = useState<{ path: typeof projectPath; value: ModelSettings | undefined }>()
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            void fetchModelSettings(projectPath || undefined).then(view => {
                if (!cancelled) setSettings({ path: projectPath, value: view.effective ?? undefined })
            }).catch(() => { if (!cancelled) setSettings(undefined) })
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
    return settings?.path === projectPath ? settings?.value : undefined
}
