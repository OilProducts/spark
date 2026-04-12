import type { ReactNode } from 'react'

import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import {
    SHARED_WEBHOOK_ENDPOINT,
    type TriggerFormState,
    type TriggerSourceType,
    type TriggerTargetMode,
} from '../model/triggerForm'

function TriggerField({
    label,
    htmlFor,
    className,
    children,
}: {
    label: string
    htmlFor?: string
    className?: string
    children: ReactNode
}) {
    return (
        <Field className={className}>
            <FieldLabel htmlFor={htmlFor}>{label}</FieldLabel>
            {children}
        </Field>
    )
}

export function TriggerEditor({
    form,
    onChange,
    mode,
    protectedTrigger,
    activeProjectPath,
}: {
    form: TriggerFormState
    onChange: (value: TriggerFormState) => void
    mode: 'create' | 'edit'
    protectedTrigger: boolean
    activeProjectPath: string | null
}) {
    const sourceTypeDisabled = protectedTrigger || mode === 'edit'
    const executionTargetDisabled = protectedTrigger
    const sourceConfigurationDisabled = protectedTrigger
    const activeTargetUnavailable = !activeProjectPath
    const fieldId = (suffix: string) => `${mode}-trigger-${suffix}`

    const onTargetModeChange = (nextTargetMode: TriggerTargetMode) => {
        if (nextTargetMode === 'active') {
            if (!activeProjectPath) {
                return
            }
            onChange({
                ...form,
                targetMode: 'active',
                projectPath: activeProjectPath,
            })
            return
        }
        if (nextTargetMode === 'none') {
            onChange({
                ...form,
                targetMode: 'none',
                projectPath: '',
            })
            return
        }
        onChange({
            ...form,
            targetMode: 'custom',
            projectPath: form.targetMode === 'custom' ? form.projectPath : '',
        })
    }

    return (
        <div className="mt-4 space-y-3">
            <div className="grid gap-3 lg:grid-cols-2">
                <TriggerField label="Name" htmlFor={fieldId('name')} className="text-sm">
                    <Input
                        id={fieldId('name')}
                        value={form.name}
                        onChange={(event) => onChange({ ...form, name: event.target.value })}
                        className="text-sm"
                    />
                </TriggerField>
                <TriggerField label="Target Flow" htmlFor={fieldId('target-flow')} className="text-sm">
                    <Input
                        id={fieldId('target-flow')}
                        value={form.flowName}
                        onChange={(event) => onChange({ ...form, flowName: event.target.value })}
                        className="text-sm font-mono"
                    />
                </TriggerField>
            </div>

            <div className="grid gap-3 lg:grid-cols-3">
                <TriggerField label="Source Type" htmlFor={fieldId('source-type')} className="text-sm">
                    <NativeSelect
                        id={fieldId('source-type')}
                        value={form.sourceType}
                        onChange={(event) => onChange({ ...form, sourceType: event.target.value as TriggerSourceType })}
                        disabled={sourceTypeDisabled}
                        className="text-sm"
                    >
                        <option value="schedule">Schedule</option>
                        <option value="poll">Poll</option>
                        <option value="webhook">Webhook</option>
                        <option value="flow_event">Flow Event</option>
                    </NativeSelect>
                </TriggerField>
                <TriggerField label="Execution Target" htmlFor={fieldId('execution-target')} className="text-sm">
                    <NativeSelect
                        id={fieldId('execution-target')}
                        value={form.targetMode}
                        onChange={(event) => onTargetModeChange(event.target.value as TriggerTargetMode)}
                        disabled={executionTargetDisabled}
                        className="text-sm"
                    >
                        <option value="active" disabled={activeTargetUnavailable}>Active project</option>
                        <option value="none">No project</option>
                        <option value="custom">Other path</option>
                    </NativeSelect>
                </TriggerField>
                <Label htmlFor={fieldId('enabled')} className="flex items-end gap-2 text-sm">
                    <Checkbox
                        id={fieldId('enabled')}
                        checked={form.enabled}
                        onCheckedChange={(checked) => onChange({ ...form, enabled: checked === true })}
                    />
                    <span className="text-xs font-medium text-foreground">Enabled</span>
                </Label>
            </div>

            {form.targetMode === 'active' ? (
                <div className="rounded-md border border-border bg-background/70 px-3 py-2 text-xs text-muted-foreground">
                    {activeProjectPath
                        ? `Uses the current active project: ${activeProjectPath}`
                        : 'No active project is available. Choose "No project" or "Other path".'}
                </div>
            ) : null}

            {form.targetMode === 'custom' ? (
                <TriggerField label="Project Path" htmlFor={fieldId('project-target')} className="text-sm">
                    <Input
                        id={fieldId('project-target')}
                        value={form.projectPath}
                        onChange={(event) => onChange({ ...form, projectPath: event.target.value })}
                        disabled={protectedTrigger}
                        className="text-sm"
                    />
                </TriggerField>
            ) : null}

            {form.sourceType === 'schedule' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <TriggerField label="Schedule Kind" htmlFor={fieldId('schedule-kind')} className="text-sm">
                        <NativeSelect
                            id={fieldId('schedule-kind')}
                            value={form.scheduleKind}
                            onChange={(event) => onChange({ ...form, scheduleKind: event.target.value as 'once' | 'interval' | 'weekly' })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                        >
                            <option value="interval">Interval</option>
                            <option value="once">One Shot</option>
                            <option value="weekly">Weekly</option>
                        </NativeSelect>
                    </TriggerField>
                    {form.scheduleKind === 'interval' ? (
                        <TriggerField label="Interval Seconds" htmlFor={fieldId('schedule-interval-seconds')} className="text-sm">
                            <Input
                                id={fieldId('schedule-interval-seconds')}
                                value={form.scheduleIntervalSeconds}
                                onChange={(event) => onChange({ ...form, scheduleIntervalSeconds: event.target.value })}
                                disabled={sourceConfigurationDisabled}
                                className="text-sm"
                            />
                        </TriggerField>
                    ) : null}
                    {form.scheduleKind === 'once' ? (
                        <TriggerField label="Run At (ISO UTC)" htmlFor={fieldId('schedule-run-at')} className="text-sm lg:col-span-2">
                            <Input
                                id={fieldId('schedule-run-at')}
                                value={form.scheduleRunAt}
                                onChange={(event) => onChange({ ...form, scheduleRunAt: event.target.value })}
                                disabled={sourceConfigurationDisabled}
                                className="text-sm font-mono"
                                placeholder="2026-03-22T15:00:00Z"
                            />
                        </TriggerField>
                    ) : null}
                    {form.scheduleKind === 'weekly' ? (
                        <>
                            <TriggerField label="Weekdays" htmlFor={fieldId('schedule-weekdays')} className="text-sm">
                                <Input
                                    id={fieldId('schedule-weekdays')}
                                    value={form.scheduleWeekdays}
                                    onChange={(event) => onChange({ ...form, scheduleWeekdays: event.target.value })}
                                    disabled={sourceConfigurationDisabled}
                                    className="text-sm"
                                    placeholder="mon,wed,fri"
                                />
                            </TriggerField>
                            <div className="grid grid-cols-2 gap-3">
                                <TriggerField label="Hour" htmlFor={fieldId('schedule-hour')} className="text-sm">
                                    <Input
                                        id={fieldId('schedule-hour')}
                                        value={form.scheduleHour}
                                        onChange={(event) => onChange({ ...form, scheduleHour: event.target.value })}
                                        disabled={sourceConfigurationDisabled}
                                        className="text-sm"
                                    />
                                </TriggerField>
                                <TriggerField label="Minute" htmlFor={fieldId('schedule-minute')} className="text-sm">
                                    <Input
                                        id={fieldId('schedule-minute')}
                                        value={form.scheduleMinute}
                                        onChange={(event) => onChange({ ...form, scheduleMinute: event.target.value })}
                                        disabled={sourceConfigurationDisabled}
                                        className="text-sm"
                                    />
                                </TriggerField>
                            </div>
                        </>
                    ) : null}
                </div>
            ) : null}

            {form.sourceType === 'poll' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <TriggerField label="Poll URL" htmlFor={fieldId('poll-url')} className="text-sm lg:col-span-2">
                        <Input
                            id={fieldId('poll-url')}
                            value={form.pollUrl}
                            onChange={(event) => onChange({ ...form, pollUrl: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm font-mono"
                        />
                    </TriggerField>
                    <TriggerField label="Interval Seconds" htmlFor={fieldId('poll-interval-seconds')} className="text-sm">
                        <Input
                            id={fieldId('poll-interval-seconds')}
                            value={form.pollIntervalSeconds}
                            onChange={(event) => onChange({ ...form, pollIntervalSeconds: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                        />
                    </TriggerField>
                    <TriggerField label="Items Path" htmlFor={fieldId('poll-items-path')} className="text-sm">
                        <Input
                            id={fieldId('poll-items-path')}
                            value={form.pollItemsPath}
                            onChange={(event) => onChange({ ...form, pollItemsPath: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                        />
                    </TriggerField>
                    <TriggerField label="Item ID Path" htmlFor={fieldId('poll-item-id-path')} className="text-sm">
                        <Input
                            id={fieldId('poll-item-id-path')}
                            value={form.pollItemIdPath}
                            onChange={(event) => onChange({ ...form, pollItemIdPath: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                        />
                    </TriggerField>
                    <TriggerField label="Headers JSON" htmlFor={fieldId('poll-headers-json')} className="text-sm lg:col-span-2">
                        <Textarea
                            id={fieldId('poll-headers-json')}
                            value={form.pollHeadersText}
                            onChange={(event) => onChange({ ...form, pollHeadersText: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="min-h-24 font-mono text-xs"
                        />
                    </TriggerField>
                </div>
            ) : null}

            {form.sourceType === 'flow_event' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <TriggerField label="Observed Flow" htmlFor={fieldId('flow-event-flow-name')} className="text-sm">
                        <Input
                            id={fieldId('flow-event-flow-name')}
                            value={form.flowEventFlowName}
                            onChange={(event) => onChange({ ...form, flowEventFlowName: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm font-mono"
                            placeholder="Leave blank for any observed flow"
                        />
                    </TriggerField>
                    <TriggerField label="Terminal Statuses" htmlFor={fieldId('flow-event-statuses')} className="text-sm">
                        <Input
                            id={fieldId('flow-event-statuses')}
                            value={form.flowEventStatuses}
                            onChange={(event) => onChange({ ...form, flowEventStatuses: event.target.value })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                            placeholder="completed,failed"
                        />
                    </TriggerField>
                </div>
            ) : null}

            {form.sourceType === 'webhook' ? (
                <div className="rounded-md border border-border bg-background/70 px-3 py-2 text-sm text-muted-foreground">
                    Webhook triggers use the shared ingress endpoint at <code>{SHARED_WEBHOOK_ENDPOINT}</code>. The key and secret are generated automatically.
                </div>
            ) : null}

            <TriggerField label="Static Context JSON" htmlFor={fieldId('static-context-json')} className="text-sm">
                <Textarea
                    id={fieldId('static-context-json')}
                    value={form.staticContextText}
                    onChange={(event) => onChange({ ...form, staticContextText: event.target.value })}
                    disabled={protectedTrigger}
                    className="min-h-24 font-mono text-xs"
                />
            </TriggerField>
        </div>
    )
}
