import type { TriggerResponse, TriggerSourceType } from '@/lib/workspaceClient'

export type TriggerFormState = {
    name: string
    enabled: boolean
    sourceType: TriggerSourceType
    flowName: string
    projectPath: string
    staticContextText: string
    scheduleKind: 'once' | 'interval' | 'weekly'
    scheduleRunAt: string
    scheduleIntervalSeconds: string
    scheduleWeekdays: string
    scheduleHour: string
    scheduleMinute: string
    pollUrl: string
    pollIntervalSeconds: string
    pollHeadersText: string
    pollItemsPath: string
    pollItemIdPath: string
    flowEventFlowName: string
    flowEventStatuses: string
}

export const EMPTY_TRIGGER_FORM: TriggerFormState = {
    name: '',
    enabled: true,
    sourceType: 'schedule',
    flowName: '',
    projectPath: '',
    staticContextText: '{}',
    scheduleKind: 'interval',
    scheduleRunAt: '',
    scheduleIntervalSeconds: '300',
    scheduleWeekdays: 'mon,fri',
    scheduleHour: '9',
    scheduleMinute: '0',
    pollUrl: 'https://example.com/data.json',
    pollIntervalSeconds: '300',
    pollHeadersText: '{}',
    pollItemsPath: 'items',
    pollItemIdPath: 'id',
    flowEventFlowName: '',
    flowEventStatuses: 'completed,failed',
}

export const SHARED_WEBHOOK_ENDPOINT = '/workspace/api/webhooks'

export function formatTriggerTimestamp(value?: string | null): string {
    if (!value) return 'Never'
    const parsed = new Date(value)
    if (Number.isNaN(parsed.getTime())) return value
    return parsed.toLocaleString()
}

export function triggerSourceSummary(trigger: TriggerResponse): string {
    if (trigger.source_type === 'schedule') {
        const kind = typeof trigger.source.kind === 'string' ? trigger.source.kind : 'schedule'
        return `Schedule · ${kind}`
    }
    if (trigger.source_type === 'poll') {
        return `Poll · ${String(trigger.source.url ?? '')}`
    }
    if (trigger.source_type === 'webhook') {
        return `Webhook · key ${String(trigger.source.webhook_key ?? '')}`
    }
    if (trigger.source_type === 'workspace_event') {
        return `System event · ${String(trigger.source.event_name ?? '')}`
    }
    return `Flow event · ${String(trigger.source.flow_name ?? 'any flow')}`
}

export function buildTriggerSourcePayload(form: TriggerFormState): Record<string, unknown> {
    if (form.sourceType === 'schedule') {
        if (form.scheduleKind === 'once') {
            return {
                kind: 'once',
                run_at: form.scheduleRunAt.trim(),
            }
        }
        if (form.scheduleKind === 'interval') {
            return {
                kind: 'interval',
                interval_seconds: parseInt(form.scheduleIntervalSeconds, 10),
            }
        }
        return {
            kind: 'weekly',
            weekdays: form.scheduleWeekdays.split(',').map((entry) => entry.trim().toLowerCase()).filter(Boolean),
            hour: parseInt(form.scheduleHour, 10),
            minute: parseInt(form.scheduleMinute, 10),
        }
    }
    if (form.sourceType === 'poll') {
        return {
            url: form.pollUrl.trim(),
            interval_seconds: parseInt(form.pollIntervalSeconds, 10),
            headers: JSON.parse(form.pollHeadersText || '{}') as Record<string, unknown>,
            items_path: form.pollItemsPath.trim(),
            item_id_path: form.pollItemIdPath.trim(),
        }
    }
    if (form.sourceType === 'flow_event') {
        return {
            flow_name: form.flowEventFlowName.trim() || null,
            statuses: form.flowEventStatuses.split(',').map((entry) => entry.trim().toLowerCase()).filter(Boolean),
        }
    }
    return {}
}

export function buildTriggerActionPayload(form: TriggerFormState): Record<string, unknown> {
    return {
        flow_name: form.flowName.trim(),
        project_path: form.projectPath.trim() || null,
        static_context: JSON.parse(form.staticContextText || '{}') as Record<string, unknown>,
    }
}

export function triggerToFormState(trigger: TriggerResponse): TriggerFormState {
    const source = trigger.source
    return {
        name: trigger.name,
        enabled: trigger.enabled,
        sourceType: trigger.source_type,
        flowName: trigger.action.flow_name,
        projectPath: trigger.action.project_path ?? '',
        staticContextText: JSON.stringify(trigger.action.static_context ?? {}, null, 2),
        scheduleKind: (typeof source.kind === 'string' ? source.kind : 'interval') as 'once' | 'interval' | 'weekly',
        scheduleRunAt: typeof source.run_at === 'string' ? source.run_at : '',
        scheduleIntervalSeconds: `${typeof source.interval_seconds === 'number' ? source.interval_seconds : 300}`,
        scheduleWeekdays: Array.isArray(source.weekdays) ? source.weekdays.map((entry) => String(entry)).join(',') : 'mon,fri',
        scheduleHour: `${typeof source.hour === 'number' ? source.hour : 9}`,
        scheduleMinute: `${typeof source.minute === 'number' ? source.minute : 0}`,
        pollUrl: typeof source.url === 'string' ? source.url : 'https://example.com/data.json',
        pollIntervalSeconds: `${typeof source.interval_seconds === 'number' ? source.interval_seconds : 300}`,
        pollHeadersText: JSON.stringify(source.headers ?? {}, null, 2),
        pollItemsPath: typeof source.items_path === 'string' ? source.items_path : 'items',
        pollItemIdPath: typeof source.item_id_path === 'string' ? source.item_id_path : 'id',
        flowEventFlowName: typeof source.flow_name === 'string' ? source.flow_name : '',
        flowEventStatuses: Array.isArray(source.statuses) ? source.statuses.map((entry) => String(entry)).join(',') : 'completed,failed',
    }
}
