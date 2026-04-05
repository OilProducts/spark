import { type StateCreator } from 'zustand'
import { createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import type { AppState } from './store-types'
import type { TriggersSessionSlice, TriggersSessionState } from './viewSessionTypes'

const DEFAULT_TRIGGERS_SESSION_STATE: TriggersSessionState = {
    status: 'idle',
    error: null,
    triggers: [],
    selectedTriggerId: null,
    scopeFilter: 'all',
    revealedWebhookSecrets: {},
    newTriggerDraft: {
        form: createEmptyTriggerForm(null),
        targetBehavior: 'default',
    },
    editTriggerDraftsByTriggerId: {},
}

export const createTriggersSessionSlice: StateCreator<AppState, [], [], TriggersSessionSlice> = (set) => ({
    triggersSession: DEFAULT_TRIGGERS_SESSION_STATE,
    updateTriggersSession: (patch) =>
        set((state) => ({
            triggersSession: {
                ...state.triggersSession,
                ...patch,
            },
        })),
    setTriggersSessionNewDraft: (draft) =>
        set((state) => ({
            triggersSession: {
                ...state.triggersSession,
                newTriggerDraft: draft,
            },
        })),
    setTriggersSessionEditDraft: (triggerId, draft) =>
        set((state) => ({
            triggersSession: {
                ...state.triggersSession,
                editTriggerDraftsByTriggerId: draft
                    ? {
                        ...state.triggersSession.editTriggerDraftsByTriggerId,
                        [triggerId]: draft,
                    }
                    : Object.fromEntries(
                        Object.entries(state.triggersSession.editTriggerDraftsByTriggerId).filter(([id]) => id !== triggerId),
                    ),
            },
        })),
})
