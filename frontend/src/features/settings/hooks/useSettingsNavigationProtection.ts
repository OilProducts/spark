import { useEffect } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'

export function useSettingsNavigationProtection(dirty: boolean, pending = false) {
    const { confirm } = useDialogController()
    useEffect(() => {
        if (!dirty && !pending) return
        let confirming = false
        let transitions: Array<() => void> = []
        const unload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
        const navigate = (event: Event) => {
            if (event.defaultPrevented) return
            event.preventDefault()
            if (pending) return
            transitions.push((event as CustomEvent<{ proceed: () => void }>).detail.proceed)
            if (confirming) return
            confirming = true
            void confirm({ title: 'Discard unsaved settings?', description: 'Your unsaved settings will be lost.', confirmLabel: 'Discard and leave', cancelLabel: 'Keep editing' })
                .then((accepted) => { if (accepted) transitions.forEach((proceed) => proceed()) })
                .finally(() => { confirming = false; transitions = [] })
        }
        window.addEventListener('beforeunload', unload)
        window.addEventListener('spark:before-navigation', navigate)
        return () => {
            window.removeEventListener('beforeunload', unload)
            window.removeEventListener('spark:before-navigation', navigate)
        }
    }, [dirty, pending, confirm])
}
