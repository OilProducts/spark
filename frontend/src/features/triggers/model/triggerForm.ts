import type { TriggerResponse, TriggerSourceType } from '@/lib/workspaceClient'
import { normalizeProjectPath } from '@/lib/projectPaths'
import type { RegisteredProject } from '@/state/store-types'
import { isUnregisteredTarget, projectLabel } from '@/features/projects/model/projectChoices'
import { buildLaunchContextFromValues, launchContextToFormValues, type LaunchInputDefinition } from '@/lib/flowContracts'

export type { TriggerSourceType }

export type TriggerFormState = {
    name: string
    enabled: boolean
    sourceType: TriggerSourceType
    actionMode: string
    missionId: string
    roleName: string
    flowName: string
    /** The project the trigger belongs to; chosen only on create. */
    projectPath: string
    staticContextText: string
    /** The flow's launch inputs as typed, by input key; saved into the static context under those keys. */
    launchInputValues: Record<string, string>
    scheduleKind: 'once' | 'interval' | 'weekly'
    /** A `datetime-local` value, in the browser's time zone. */
    scheduleRunAt: string
    /** A count of `scheduleIntervalUnit`s; see `intervalSeconds`. */
    scheduleInterval: string
    scheduleIntervalUnit: number
    /** Keys from `WEEKDAYS`, in week order. */
    scheduleWeekdays: string[]
    /** A `time` input value, "HH:MM", in `scheduleTimezone`. */
    scheduleTime: string
    /** The IANA time zone of the weekly days and time; a new trigger takes the browser's. */
    scheduleTimezone: string
    pollUrl: string
    pollInterval: string
    pollIntervalUnit: number
    pollHeadersText: string
    pollItemsPath: string
    pollItemIdPath: string
    flowEventFlowName: string
    flowEventStatuses: string
}

const BASE_TRIGGER_FORM: Omit<TriggerFormState, 'projectPath' | 'scheduleTimezone'> = {
    name: '',
    enabled: true,
    sourceType: 'schedule',
    actionMode: 'static',
    missionId: '',
    roleName: '',
    flowName: '',
    staticContextText: '',
    launchInputValues: {},
    scheduleKind: 'interval',
    scheduleRunAt: '',
    scheduleInterval: '5',
    scheduleIntervalUnit: 60,
    scheduleWeekdays: ['mon', 'fri'],
    scheduleTime: '09:00',
    pollUrl: 'https://example.com/data.json',
    pollInterval: '5',
    pollIntervalUnit: 60,
    pollHeadersText: '',
    pollItemsPath: 'items',
    pollItemIdPath: 'id',
    flowEventFlowName: '',
    flowEventStatuses: 'completed,failed',
}

export const browserTimeZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone

export const createEmptyTriggerForm = (projectPath: string | null): TriggerFormState => ({
    ...BASE_TRIGGER_FORM,
    projectPath: projectPath || '',
    scheduleTimezone: browserTimeZone(),
})

const WEEKDAY_NAMES = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
/** Each day of the week: its key as the server stores it ("mon"), short label and full name. */
export const WEEKDAYS = WEEKDAY_NAMES.map((name) => ({ key: name.slice(0, 3).toLowerCase(), label: name.slice(0, 3), name }))

export const SHARED_WEBHOOK_ENDPOINT = '/workspace/api/webhooks'

const TIME_FORMAT = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
const DAY_FORMAT = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric' })
const DAY_YEAR_FORMAT = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' })
const NEARBY_DAYS = new Map([[-1, 'Yesterday'], [0, 'Today'], [1, 'Tomorrow']])
const localMidnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()

/** "Today at 8:13 PM", "Tomorrow at 9:00 AM", "Mon, Nov 16 at 8:13 PM"; the year only when it isn't this year. */
export function formatTriggerTimestamp(value?: string | null, now = Date.now()): string {
    if (!value) return 'Never'
    const date = new Date(value)
    if (Number.isNaN(date.getTime())) return value
    const today = new Date(now)
    // Rounded: a day with a daylight-saving change is 23 or 25 hours long.
    const days = Math.round((localMidnight(date) - localMidnight(today)) / 86_400_000)
    const day = NEARBY_DAYS.get(days) ?? (date.getFullYear() === today.getFullYear() ? DAY_FORMAT : DAY_YEAR_FORMAT).format(date)
    return `${day} at ${TIME_FORMAT.format(date)}`
}

/** A trigger result in a word: "Succeeded", "Failed", else the status capitalized. */
/**
 * What is wrong with a trigger, for its row and the views that flag it; null when nothing is.
 * A disabled trigger fires no more, so only a failed activation still counts for it.
 */
export function triggerProblem(trigger: Pick<TriggerResponse, 'enabled' | 'state'>): { label: string; detail: string } | null {
    if (trigger.state.activation_error) return { label: 'Not running', detail: trigger.state.activation_error }
    if (trigger.enabled && (trigger.state.last_result === 'failed' || trigger.state.last_result === 'failure')) {
        return { label: 'Last firing failed', detail: trigger.state.last_error || 'Its last firing failed.' }
    }
    return null
}

export function formatTriggerResult(status?: string | null): string {
    if (!status) return 'Never'
    if (status === 'success') return 'Succeeded'
    if (status === 'failure' || status === 'failed') return 'Failed'
    const text = status.replace(/_/g, ' ')
    return text.charAt(0).toUpperCase() + text.slice(1)
}

/** Units for entering an interval, in seconds, largest first. */
export const INTERVAL_UNITS = [
    { seconds: 86400, label: 'days' },
    { seconds: 3600, label: 'hours' },
    { seconds: 60, label: 'minutes' },
    { seconds: 1, label: 'seconds' },
] as const

/** An interval in seconds as a count of the largest unit that divides it evenly. */
export function splitInterval(seconds: number): { value: string; unit: number } {
    const unit = INTERVAL_UNITS.find((entry) => seconds % entry.seconds === 0)?.seconds ?? 1
    return { value: String(seconds / unit), unit }
}

/** Whole seconds for a count of `unit`, or null unless it comes to at least a second. */
export function intervalSeconds(value: string, unit: number): number | null {
    const seconds = Math.round(Number(value) * unit)
    return Number.isFinite(seconds) && seconds > 0 ? seconds : null
}

/** "1 day", "5 minutes". */
function intervalText(seconds: number): string {
    const { value, unit } = splitInterval(seconds)
    const label = INTERVAL_UNITS.find((entry) => entry.seconds === unit)?.label ?? 'seconds'
    return `${value} ${value === '1' ? label.slice(0, -1) : label}`
}

/** "Every day", "Every 5 minutes". */
const describeInterval = (seconds: number) => `Every ${intervalText(seconds).replace(/^1 /, '')}`

/** A new interval trigger first runs one interval after it is saved. */
export function firstIntervalRunHint(seconds: number, now = Date.now()): string {
    const at = new Date(now + seconds * 1000)
    // A huge interval runs past the last date a Date can hold.
    const first = Number.isNaN(at.getTime()) ? '' : ` (${formatTriggerTimestamp(at.toISOString(), now)})`
    return `First run in ${intervalText(seconds)}${first}, then ${describeInterval(seconds).toLowerCase()}.`
}

/** "Mon, Wed, Fri at 9:00 AM", naming the time zone when it isn't the browser's. */
function describeWeekly(source: Record<string, unknown>): string {
    const { weekdays, hour, minute, timezone = 'UTC' } = source
    if (!Array.isArray(weekdays) || typeof hour !== 'number' || typeof minute !== 'number') return 'Weekly'
    const days = weekdays.length === WEEKDAYS.length
        ? 'Every day'
        : WEEKDAYS.filter((day) => weekdays.includes(day.key)).map((day) => day.label).join(', ')
    const zone = timezone === browserTimeZone() ? '' : ` (${String(timezone)})`
    return `${days} at ${TIME_FORMAT.format(new Date(2000, 0, 1, hour, minute))}${zone}`
}

/** When the trigger starts, in a few plain words. */
export function triggerSourceSummary(trigger: TriggerResponse): string {
    const { source } = trigger
    if (trigger.source_type === 'schedule') {
        if (source.kind === 'interval' && typeof source.interval_seconds === 'number') return describeInterval(source.interval_seconds)
        if (source.kind === 'weekly') return describeWeekly(source)
        return source.kind === 'once' ? 'Once' : 'Schedule'
    }
    if (trigger.source_type === 'poll') {
        return `Checks ${String(source.url ?? 'a URL')}`
    }
    if (trigger.source_type === 'webhook') {
        return 'Webhook'
    }
    return typeof source.flow_name === 'string' && source.flow_name ? `When ${source.flow_name} finishes` : 'When any flow finishes'
}

type Registry = Record<string, RegisteredProject>

/** The project the trigger belongs to, by name; one no longer registered says so. */
export function triggerProjectSummary(trigger: TriggerResponse, registry: Registry): string {
    const label = projectLabel(registry, normalizeProjectPath(trigger.project_path))
    return isUnregisteredTarget(registry, trigger.project_path)
        ? `Unregistered project · ${label}`
        : label
}

/** A `datetime-local` value for an ISO timestamp, in the browser's time zone. */
export function toLocalDateTimeInput(iso: string): string {
    const time = new Date(iso).getTime()
    return Number.isNaN(time) ? '' : new Date(time - new Date(time).getTimezoneOffset() * 60_000).toISOString().slice(0, 16)
}

const splitList = (text: string) => text.split(',').map((entry) => entry.trim().toLowerCase()).filter(Boolean)
const parseJsonObject = (text: string): Record<string, unknown> | null => {
    if (!text.trim()) return {}
    try {
        const value: unknown = JSON.parse(text)
        return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
    } catch {
        return null
    }
}
const TIME_INPUT = /^([01]\d|2[0-3]):[0-5]\d$/

export type TriggerFieldErrors = Partial<Record<keyof TriggerFormState, string>>

/**
 * What is wrong with each of the flow's launch inputs, by key. A disabled trigger
 * starts no run, so it may leave a required input unset; a value it sets must still be valid.
 */
export const triggerLaunchInputErrors = (form: TriggerFormState, inputs: LaunchInputDefinition[]) => (
    buildLaunchContextFromValues(form.enabled ? inputs : inputs.map((entry) => ({ ...entry, required: false })), form.launchInputValues).errors
)

/**
 * What must be fixed before saving, by field, in plain words. A protected
 * trigger only edits its name and flow. `inputs` are the chosen flow's launch inputs.
 */
export function validateTriggerForm(form: TriggerFormState, protectedTrigger = false, inputs: LaunchInputDefinition[] = []): TriggerFieldErrors {
    const errors: TriggerFieldErrors = {}
    const json = (key: 'staticContextText' | 'pollHeadersText', example: string) => {
        if (!parseJsonObject(form[key])) errors[key] = `This isn't a valid JSON object. Write it in braces, like ${example}, or leave it empty.`
    }
    if (!form.name.trim()) errors.name = 'Give the trigger a name.'
    if (form.actionMode === 'static' && !form.flowName.trim()) errors.flowName = 'Choose the flow to start.'
    if (protectedTrigger) return errors
    if (form.actionMode === 'role' && !form.roleName.trim()) errors.roleName = 'Enter the standing role to send to.'
    if (form.actionMode === 'static') json('staticContextText', '{"branch": "main"}')
    if (form.actionMode === 'static' && Object.keys(triggerLaunchInputErrors(form, inputs)).length) {
        errors.launchInputValues = 'Fill in what each run needs.'
    }
    if (form.sourceType === 'schedule' && form.scheduleKind === 'interval' && !intervalSeconds(form.scheduleInterval, form.scheduleIntervalUnit)) {
        errors.scheduleInterval = 'Enter a number greater than zero.'
    }
    if (form.sourceType === 'schedule' && form.scheduleKind === 'once' && Number.isNaN(new Date(form.scheduleRunAt).getTime())) {
        errors.scheduleRunAt = 'Pick a date and time.'
    }
    if (form.sourceType === 'schedule' && form.scheduleKind === 'weekly') {
        if (!form.scheduleWeekdays.length) errors.scheduleWeekdays = 'Pick at least one day.'
        if (!TIME_INPUT.test(form.scheduleTime)) errors.scheduleTime = 'Pick a time.'
    }
    if (form.sourceType === 'poll') {
        if (!/^https?:\/\/\S+$/.test(form.pollUrl.trim())) errors.pollUrl = 'Enter a web address starting with http:// or https://.'
        if (!intervalSeconds(form.pollInterval, form.pollIntervalUnit)) errors.pollInterval = 'Enter a number greater than zero.'
        if (!form.pollItemsPath.trim()) errors.pollItemsPath = 'Enter where the list of items is, such as items.'
        if (!form.pollItemIdPath.trim()) errors.pollItemIdPath = 'Enter the field that identifies each item, such as id.'
        json('pollHeadersText', '{"Authorization": "Bearer …"}')
    }
    return errors
}

/** The source as the server takes it; call only on a form `validateTriggerForm` accepts. */
export function buildTriggerSourcePayload(form: TriggerFormState): Record<string, unknown> {
    if (form.sourceType === 'schedule') {
        if (form.scheduleKind === 'once') {
            return {
                kind: 'once',
                run_at: new Date(form.scheduleRunAt).toISOString().replace(/\.\d{3}Z$/, 'Z'),
            }
        }
        if (form.scheduleKind === 'interval') {
            return {
                kind: 'interval',
                interval_seconds: intervalSeconds(form.scheduleInterval, form.scheduleIntervalUnit),
            }
        }
        return {
            kind: 'weekly',
            weekdays: form.scheduleWeekdays,
            hour: Number(form.scheduleTime.slice(0, 2)),
            minute: Number(form.scheduleTime.slice(3, 5)),
            timezone: form.scheduleTimezone,
        }
    }
    if (form.sourceType === 'poll') {
        return {
            url: form.pollUrl.trim(),
            interval_seconds: intervalSeconds(form.pollInterval, form.pollIntervalUnit),
            headers: parseJsonObject(form.pollHeadersText),
            items_path: form.pollItemsPath.trim(),
            item_id_path: form.pollItemIdPath.trim(),
        }
    }
    if (form.sourceType === 'flow_event') {
        return {
            flow_name: form.flowEventFlowName.trim() || null,
            statuses: splitList(form.flowEventStatuses),
        }
    }
    return {}
}

/** The action as the server takes it; launch inputs join the static context under their keys. */
export function buildTriggerActionPayload(form: TriggerFormState, inputs: LaunchInputDefinition[] = []): Record<string, unknown> {
    if (form.actionMode === 'mission') return { mode: 'mission', mission_id: form.missionId }
    if (form.actionMode === 'role') return { mode: 'role', role: form.roleName.trim() }

    return {
        flow_name: form.flowName.trim(),
        static_context: {
            ...parseJsonObject(form.staticContextText),
            ...buildLaunchContextFromValues(inputs, form.launchInputValues).launchContext,
        },
    }
}

const pad = (value: number) => String(value).padStart(2, '0')
const jsonText = (value: unknown) => value && typeof value === 'object' && Object.keys(value).length ? JSON.stringify(value, null, 2) : ''

/**
 * The trigger as a form. The flow's launch inputs (`inputs`, when known)
 * come out of the static context into their own fields; the rest stays extra data.
 */
export function triggerToFormState(trigger: TriggerResponse, inputs: LaunchInputDefinition[] = []): TriggerFormState {
    const source = trigger.source
    const staticContext = { ...trigger.action.static_context }
    const setInputs = inputs.filter((entry) => entry.key in staticContext)
    const launchInputValues = launchContextToFormValues(setInputs, staticContext)
    setInputs.forEach((entry) => delete staticContext[entry.key])
    const interval = splitInterval(typeof source.interval_seconds === 'number' ? source.interval_seconds : 300)
    return {
        name: trigger.name,
        enabled: trigger.enabled,
        sourceType: trigger.source_type,
        actionMode: trigger.action.mode ?? 'static',
        missionId: trigger.action.mission_id ?? '',
        roleName: trigger.action.role ?? '',
        flowName: trigger.action.flow_name,
        projectPath: trigger.project_path,
        staticContextText: jsonText(staticContext),
        launchInputValues,
        scheduleKind: (typeof source.kind === 'string' ? source.kind : 'interval') as 'once' | 'interval' | 'weekly',
        scheduleRunAt: typeof source.run_at === 'string' ? toLocalDateTimeInput(source.run_at) : '',
        scheduleInterval: interval.value,
        scheduleIntervalUnit: interval.unit,
        scheduleWeekdays: Array.isArray(source.weekdays) ? source.weekdays.map((entry) => String(entry)) : ['mon', 'fri'],
        scheduleTime: typeof source.hour === 'number' && typeof source.minute === 'number' ? `${pad(source.hour)}:${pad(source.minute)}` : '09:00',
        // A weekly trigger saved without a zone runs in UTC.
        scheduleTimezone: typeof source.timezone === 'string' ? source.timezone : source.kind === 'weekly' ? 'UTC' : browserTimeZone(),
        pollUrl: typeof source.url === 'string' ? source.url : 'https://example.com/data.json',
        pollInterval: interval.value,
        pollIntervalUnit: interval.unit,
        pollHeadersText: jsonText(source.headers),
        pollItemsPath: typeof source.items_path === 'string' ? source.items_path : 'items',
        pollItemIdPath: typeof source.item_id_path === 'string' ? source.item_id_path : 'id',
        flowEventFlowName: typeof source.flow_name === 'string' ? source.flow_name : '',
        flowEventStatuses: Array.isArray(source.statuses) ? source.statuses.map((entry) => String(entry)).join(',') : 'completed,failed',
    }
}
