import { describe, expect, it } from 'vitest'

import { parseTriggerResponse } from '@/lib/workspaceClient'

import { buildTriggerActionPayload, createEmptyTriggerForm, triggerToFormState } from '../triggerForm'

describe('trigger form role actions', () => {
    it('builds a role action with its role and project and no flow or static context', () => {
        const form = { ...createEmptyTriggerForm('/project'), actionMode: 'role', roleName: ' chief-of-staff ', targetMode: 'custom' as const, projectPath: '/project' }
        expect(buildTriggerActionPayload(form)).toEqual({ mode: 'role', role: 'chief-of-staff', project_path: '/project' })
    })

    it('loads a role trigger back into the form', () => {
        const trigger = parseTriggerResponse({
            id: 't', name: 'Failures', enabled: true, protected: false, revision: 'r', source_type: 'flow_event',
            created_at: '', updated_at: '',
            action: { mode: 'role', role: 'chief-of-staff', project_path: '/project' },
            source: { flow_name: null, statuses: ['failed', 'validation_error'] },
            state: { recent_history: [] },
        })
        const form = triggerToFormState(trigger, {})
        expect(form.actionMode).toBe('role')
        expect(form.roleName).toBe('chief-of-staff')
        expect(buildTriggerActionPayload(form)).toEqual({ mode: 'role', role: 'chief-of-staff', project_path: '/project' })
    })
})
