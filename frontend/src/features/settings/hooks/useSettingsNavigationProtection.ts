import { useEffect, useState } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'
import type { NavigationRequest } from '@/lib/activeEnvironment'
import { requestNavigation } from '@/state/workspaceSlice'

/**
 * Asks before a navigation or page unload drops unsaved edits; `subject` names what is lost, `onDiscard` drops what
 * outlives the view. Returns this editor's identity, which a leave it already agreed to carries among its `approved`.
 */
export function useSettingsNavigationProtection(dirty: boolean, pending = false, subject = 'settings', onDiscard?: () => void) {
    const { confirm } = useDialogController()
    const [guard] = useState({})
    useEffect(() => {
        if (!dirty && !pending) return
        let confirming = false
        let transitions: NavigationRequest[] = []
        const unload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
        const navigate = (event: Event) => {
            const leave = (event as CustomEvent<NavigationRequest>).detail
            if (event.defaultPrevented || (!pending && leave.approved?.includes(guard))) return
            event.preventDefault()
            if (pending) return leave.cancel?.()
            transitions.push(leave)
            if (confirming) return
            confirming = true
            // It closes unanswered once none of its leaves goes ahead, such as a switch to a destination since removed.
            const live = () => transitions.some((transition) => transition.live?.() ?? true)
            void confirm({ title: `Discard unsaved ${subject}?`, description: `Your unsaved ${subject} will be lost.`, confirmLabel: 'Discard and leave', cancelLabel: 'Keep editing', confirmVariant: 'destructive', leaving: true, live })
                .then((accepted) => {
                    if (!accepted) return transitions.forEach(({ cancel }) => cancel?.())
                    // Every other editor the leave would drop, such as open flow files behind Settings in a switch to
                    // another environment, still asks; it goes, dropping what outlives this view, once all agree.
                    // ponytail: every editor with unsaved edits asks about any leave, even one that keeps it mounted;
                    // scope leaves to the editors they drop if that asks too often.
                    transitions.forEach((leave) => requestNavigation(() => { onDiscard?.(); leave.proceed() }, leave.cancel, { ...leave, approved: [...(leave.approved ?? []), guard] }))
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
    }, [dirty, pending, subject, confirm, onDiscard, guard])
    return guard
}
