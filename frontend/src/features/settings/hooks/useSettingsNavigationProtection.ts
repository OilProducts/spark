import { useEffect } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'
import { requestNavigation } from '@/state/workspaceSlice'

/** Asks before a navigation or page unload drops unsaved edits; `subject` names what is lost, `onDiscard` drops what outlives the view. */
export function useSettingsNavigationProtection(dirty: boolean, pending = false, subject = 'settings', onDiscard?: () => void) {
    const { confirm } = useDialogController()
    useEffect(() => {
        if (!dirty && !pending) return
        let confirming = false
        // Set once this editor agreed to a leave, until the other editors it passed the leave on to settle it.
        let approved = false
        let transitions: Array<{ proceed: () => void; cancel?: () => void }> = []
        const unload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
        const navigate = (event: Event) => {
            if (event.defaultPrevented || approved) return
            event.preventDefault()
            const leave = (event as CustomEvent<{ proceed: () => void; cancel?: () => void }>).detail
            if (pending) return leave.cancel?.()
            transitions.push(leave)
            if (confirming) return
            confirming = true
            void confirm({ title: `Discard unsaved ${subject}?`, description: `Your unsaved ${subject} will be lost.`, confirmLabel: 'Discard and leave', cancelLabel: 'Keep editing', confirmVariant: 'destructive', leaving: true })
                .then((accepted) => {
                    if (!accepted) return transitions.forEach(({ cancel }) => cancel?.())
                    // Every other editor the leave would drop, such as open flow files behind Settings in a switch to
                    // another environment, still asks; it goes, dropping what outlives this view, once all agree.
                    // ponytail: every editor with unsaved edits asks about any leave, even one that keeps it mounted;
                    // scope leaves to the editors they drop if that asks too often.
                    approved = true
                    const settle = (then?: () => void) => () => { approved = false; then?.() }
                    transitions.forEach(({ proceed, cancel }) => requestNavigation(settle(() => { onDiscard?.(); proceed() }), settle(cancel)))
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
