import type { TriggerResponse, TriggerSourceType } from '@/lib/workspaceClient'
import { normalizeProjectPath } from '@/lib/projectPaths'

export type { TriggerSourceType }

export type TriggerTargetMode = 'active' | 'none' | 'custom'

export type TriggerFormState = {
    name: string
    enabled: boolean
    sourceType: TriggerSourceType
    actionMode: string
    missionId: string
    flowName: string
    targetMode: TriggerTargetMode
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

const BASE_TRIGGER_FORM: Omit<TriggerFormState, 'targetMode' | 'projectPath'> = {
    name: '',
    enabled: true,
    sourceType: 'schedule',
    actionMode: 'static',
    missionId: '',
    flowName: '',
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

export const createEmptyTriggerForm = (activeProjectPath: string | null): TriggerFormState => ({
    ...BASE_TRIGGER_FORM,
    targetMode: activeProjectPath ? 'active' : 'none',
    projectPath: activeProjectPath || '',
})

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
    return `Flow event · ${String(trigger.source.flow_name ?? 'any flow')}`
}

const formatProjectLabel = (projectPath: string) => {
    const normalizedPath = normalizeProjectPath(projectPath)
    const segments = normalizedPath.split('/').filter(Boolean)
    return segments[segments.length - 1] || normalizedPath
}

export function triggerTargetsActiveProject(trigger: TriggerResponse, activeProjectPath: string | null): boolean {
    return Boolean(activeProjectPath) && trigger.action.project_path === activeProjectPath
}

export function triggerTargetSummary(trigger: TriggerResponse, activeProjectPath: string | null): string {
    if (!trigger.action.project_path) {
        return 'No project'
    }
    if (triggerTargetsActiveProject(trigger, activeProjectPath)) {
        return 'Targets active project'
    }
    return `Project · ${formatProjectLabel(trigger.action.project_path)}`
}

export function resolveTriggerTargetMode(
    projectPath: string | null | undefined,
    activeProjectPath: string | null,
): TriggerTargetMode {
    const normalizedProjectPath = typeof projectPath === 'string' ? projectPath.trim() : ''
    if (!normalizedProjectPath) {
        return 'none'
    }
    return activeProjectPath && normalizedProjectPath === activeProjectPath ? 'active' : 'custom'
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
    if (form.actionMode === 'mission') return { mode: 'mission', mission_id: form.missionId, project_path: form.targetMode === 'none' ? null : form.projectPath.trim() || null }

    return {
        flow_name: form.flowName.trim(),
        project_path: form.targetMode === 'none' ? null : form.projectPath.trim() || null,
        static_context: JSON.parse(form.staticContextText || '{}') as Record<string, unknown>,
    }
}

export function triggerToFormState(trigger: TriggerResponse, activeProjectPath: string | null): TriggerFormState {
    const source = trigger.source
    const projectPath = trigger.action.project_path ?? ''
    return {
        name: trigger.name,
        enabled: trigger.enabled,
        sourceType: trigger.source_type,
        actionMode: trigger.action.mode ?? 'static',
        missionId: trigger.action.mission_id ?? '',
        flowName: trigger.action.flow_name,
        targetMode: resolveTriggerTargetMode(projectPath, activeProjectPath),
        projectPath,
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
