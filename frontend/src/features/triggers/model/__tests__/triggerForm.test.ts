import { describe, expect, it } from 'vitest'

import { parseTriggerResponse } from '@/lib/workspaceClient'

import {
    buildTriggerActionPayload,
    buildTriggerSourcePayload,
    createEmptyTriggerForm,
    firstIntervalRunHint,
    formatTriggerResult,
    formatTriggerTimestamp,
    intervalSeconds,
    splitInterval,
    triggerToFormState,
    validateTriggerForm,
} from '../triggerForm'

const trigger = (source: Record<string, unknown>, action: Record<string, unknown> = { flow_name: 'nightly' }) => parseTriggerResponse({
    id: 't', project_path: '/project', name: 'T', enabled: true, protected: false, revision: 'r', source_type: 'schedule',
    created_at: '', updated_at: '', action, source, state: { recent_history: [] },
})

describe('trigger form role actions', () => {
    it('builds a role action with its role and no project, flow or static context', () => {
        const form = { ...createEmptyTriggerForm('/project'), actionMode: 'role', roleName: ' chief-of-staff ' }
        expect(buildTriggerActionPayload(form)).toEqual({ mode: 'role', role: 'chief-of-staff' })
    })

    it('loads a role trigger back into the form', () => {
        const trigger = parseTriggerResponse({
            id: 't', project_path: '/project', name: 'Failures', enabled: true, protected: false, revision: 'r', source_type: 'flow_event',
            created_at: '', updated_at: '',
            action: { mode: 'role', role: 'chief-of-staff' },
            source: { flow_name: null, statuses: ['failed', 'validation_error'] },
            state: { recent_history: [] },
        })
        const form = triggerToFormState(trigger)
        expect(form.actionMode).toBe('role')
        expect(form.roleName).toBe('chief-of-staff')
        expect(form.projectPath).toBe('/project')
        expect(buildTriggerActionPayload(form)).toEqual({ mode: 'role', role: 'chief-of-staff' })
    })
})

describe('trigger form intervals', () => {
    it('shows seconds in the largest unit that divides them, and saves them back as seconds', () => {
        expect(splitInterval(300)).toEqual({ value: '5', unit: 60 })
        expect(splitInterval(7200)).toEqual({ value: '2', unit: 3600 })
        expect(splitInterval(86400)).toEqual({ value: '1', unit: 86400 })
        expect(splitInterval(90)).toEqual({ value: '90', unit: 1 })
        expect(intervalSeconds('1.5', 3600)).toBe(5400)
        for (const bad of ['', '0', '-1', 'soon']) expect(intervalSeconds(bad, 60)).toBeNull()

        const form = triggerToFormState(trigger({ kind: 'interval', interval_seconds: 7200 }))
        expect([form.scheduleInterval, form.scheduleIntervalUnit]).toEqual(['2', 3600])
        expect(buildTriggerSourcePayload(form)).toEqual({ kind: 'interval', interval_seconds: 7200 })
    })

    it('round-trips a one-time run through the local date picker', () => {
        const form = triggerToFormState(trigger({ kind: 'once', run_at: '2030-01-01T10:00:00Z' }))
        expect(form.scheduleRunAt).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/)
        expect(buildTriggerSourcePayload(form)).toEqual({ kind: 'once', run_at: '2030-01-01T10:00:00Z' })
    })
})

describe('trigger status wording', () => {
    // A Wednesday. Times print with a narrow space before AM/PM on newer ICU.
    const now = Date.parse('2026-11-11T20:13:47')
    const plain = (text: string) => text.replace(/\s/g, ' ')
    const when = (value: string | null) => plain(formatTriggerTimestamp(value, now))

    it('names nearby days, drops seconds, and adds the year only when it is not this year', () => {
        expect(when('2026-11-11T20:13:47')).toBe('Today at 8:13 PM')
        expect(when('2026-11-12T09:00:00')).toBe('Tomorrow at 9:00 AM')
        expect(when('2026-11-10T23:59:00')).toBe('Yesterday at 11:59 PM')
        expect(when('2026-11-16T20:13:00')).toBe('Mon, Nov 16 at 8:13 PM')
        expect(when('2027-01-04T08:00:00')).toBe('Mon, Jan 4, 2027 at 8:00 AM')
        expect(when(null)).toBe('Never')
    })

    it('words a result as a capitalized word', () => {
        expect(formatTriggerResult('success')).toBe('Succeeded')
        expect(formatTriggerResult('failed')).toBe('Failed')
        expect(formatTriggerResult('failure')).toBe('Failed')
        expect(formatTriggerResult('validation_error')).toBe('Validation error')
        expect(formatTriggerResult(null)).toBe('Never')
    })

    it('says a new interval trigger first runs one interval after it is saved', () => {
        expect(plain(firstIntervalRunHint(86400, now))).toBe('First run in 1 day (Tomorrow at 8:13 PM), then every day.')
        expect(plain(firstIntervalRunHint(300, now))).toBe('First run in 5 minutes (Today at 8:18 PM), then every 5 minutes.')
        expect(firstIntervalRunHint(1e20 * 86400, now)).toMatch(/^First run in [^(]+, then every [^(]+\.$/)
    })
})

describe('trigger form validation', () => {
    const valid = { ...createEmptyTriggerForm('/project'), name: 'Nightly', flowName: 'nightly' }

    it('accepts an empty static context as optional, and keeps a saved one', () => {
        expect(validateTriggerForm(valid)).toEqual({})
        expect(buildTriggerActionPayload(valid)).toEqual({ flow_name: 'nightly', static_context: {} })
        expect(triggerToFormState(trigger({ kind: 'interval', interval_seconds: 60 })).staticContextText).toBe('')
        const saved = triggerToFormState(trigger({ kind: 'interval', interval_seconds: 60 }, { flow_name: 'nightly', static_context: { branch: 'main' } }))
        expect(buildTriggerActionPayload(saved)).toEqual({ flow_name: 'nightly', static_context: { branch: 'main' } })
    })

    it('names each field to fix in plain words', () => {
        for (const staticContextText of ['{branch: main}', '[1]', '"main"']) {
            expect(validateTriggerForm({ ...valid, staticContextText }).staticContextText).toMatch(/^This isn't a valid JSON object\./)
        }
        expect(validateTriggerForm({ ...valid, name: ' ', flowName: '' })).toEqual({ name: 'Give the trigger a name.', flowName: 'Choose the flow to start.' })
        expect(Object.keys(validateTriggerForm({ ...valid, scheduleKind: 'weekly', scheduleWeekdays: 'monday', scheduleHour: '24', scheduleMinute: '5' })))
            .toEqual(['scheduleWeekdays', 'scheduleHour'])
        expect(Object.keys(validateTriggerForm({ ...valid, sourceType: 'poll', pollUrl: 'example.com', pollHeadersText: '{', pollInterval: '0' })))
            .toEqual(['pollUrl', 'pollInterval', 'pollHeadersText'])
    })

    it('checks only the name and flow of a protected trigger, which edits nothing else', () => {
        expect(validateTriggerForm({ ...valid, flowName: '', staticContextText: '{', scheduleInterval: '0' }, true)).toEqual({ flowName: 'Choose the flow to start.' })
    })
})
