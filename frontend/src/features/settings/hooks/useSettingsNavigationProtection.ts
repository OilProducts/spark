import { useEffect } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'

/** Asks before a navigation or page unload drops unsaved edits; `subject` names what is lost, `onDiscard` drops what outlives the view. */
export function useSettingsNavigationProtection(dirty: boolean, pending = false, subject = 'settings', onDiscard?: () => void) {
    const { confirm } = useDialogController()
    useEffect(() => {
        if (!dirty && !pending) return
        let confirming = false
        let transitions: Array<{ proceed: () => void; cancel?: () => void }> = []
        const unload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
        const navigate = (event: Event) => {
            if (event.defaultPrevented) return
            event.preventDefault()
            const leave = (event as CustomEvent<{ proceed: () => void; cancel?: () => void }>).detail
            if (pending) return leave.cancel?.()
            transitions.push(leave)
            if (confirming) return
            confirming = true
            void confirm({ title: `Discard unsaved ${subject}?`, description: `Your unsaved ${subject} will be lost.`, confirmLabel: 'Discard and leave', cancelLabel: 'Keep editing', confirmVariant: 'destructive', leaving: true })
                .then((accepted) => {
                    if (!accepted) return transitions.forEach(({ cancel }) => cancel?.())
                    onDiscard?.()
                    transitions.forEach(({ proceed }) => proceed())
                })
                .finally(() => { confirming = false; transitions = [] })
        }
        window.addEventListener('beforeunload', unload)
        // Pending writes must block navigation before any dirty editor offers to discard.
        window.addEventListener('spark:before-navigation', navigate, { capture: pending })
        return () => {
            window.removeEventListener('beforeunload', unload)
            window.removeEventListener('spark:before-navigation', navigate, { capture: pending })
        }
    }, [dirty, pending, subject, confirm, onDiscard])
}
