import { useEffect, useRef, useState, type ReactNode } from 'react'
import { request, type Board, type Mission } from '@/features/missions/MissionsPanel'
import { orderProjects, projectLabel } from '@/features/projects/model/projectChoices'
import { useStore } from '@/store'
import { cn } from '@/lib/utils'

import { SegmentedControl } from '@/components/app/segmented-control'
import { Well } from '@/components/app/well'
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { groupFlows, useFlowSummaries, useOwnFlows } from '@/features/launch'
import type { WorkspaceFlowResponse } from '@/lib/api/flowsApi'
import {
    INTERVAL_UNITS,
    SHARED_WEBHOOK_ENDPOINT,
    WEEKDAYS,
    browserTimeZone,
    firstIntervalRunHint,
    intervalSeconds,
    type TriggerFieldErrors,
    type TriggerFormState,
    type TriggerSourceType,
} from '../model/triggerForm'

/** A project's flows, and which of them are its own rather than bundled with Spark (null while unknown). */
type ProjectFlows = { status: 'loading' | 'ready' | 'error'; flows: WorkspaceFlowResponse[]; own: ReadonlySet<string> | null; name: string }

/** A label, the control, then its hint and error, which the control names with `describedBy`. */
function TriggerField({
    label,
    htmlFor,
    hint,
    error,
    className,
    children,
}: {
    label: string
    htmlFor: string
    hint?: ReactNode
    error?: string
    className?: string
    children: ReactNode
}) {
    return (
        // content-start: a hint or error in one cell must not shift labels in the next.
        <Field className={cn('content-start', className)}>
            <FieldLabel htmlFor={htmlFor}>{label}</FieldLabel>
            {children}
            {hint ? <FieldDescription id={`${htmlFor}-hint`} className="text-xs">{hint}</FieldDescription> : null}
            {error ? <FieldError id={`${htmlFor}-error`}>{error}</FieldError> : null}
        </Field>
    )
}

const describedBy = (id: string, error?: string, hint?: ReactNode) => ({
    'aria-invalid': error ? true : undefined,
    'aria-describedby': [hint ? `${id}-hint` : '', error ? `${id}-error` : ''].join(' ').trim() || undefined,
})

/**
 * The project's flows by title, keeping a saved flow that is not among them.
 * If the list cannot load, the flow is typed by name instead.
 */
function FlowSelect({
    id,
    label,
    value,
    onChange,
    project,
    emptyLabel,
    hint,
    error,
    disabled,
}: {
    id: string
    label: string
    value: string
    onChange: (value: string) => void
    project: ProjectFlows
    emptyLabel: string
    hint: string
    error?: string
    disabled?: boolean
}) {
    const flow = project.flows.find((entry) => entry.name === value)
    // As in Run a flow: the project's own flows first, then Spark's bundled flows by folder.
    const { own, folders } = groupFlows(project.flows, '', project.own)
    const option = (entry: WorkspaceFlowResponse) => (
        <option key={entry.name} value={entry.name} title={entry.description || undefined}>{entry.title}</option>
    )
    const missing = Boolean(value) && project.status === 'ready' && !flow
    const note = missing
        ? <span className="text-destructive">This flow isn't in the project. Choose another.</span>
        : value ? <span className="font-mono">{value}</span> : hint
    return (
        <TriggerField label={label} htmlFor={id} hint={note} error={error} className="text-sm">
            {project.status === 'error' ? (
                <Input id={id} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} className="font-mono text-sm" {...describedBy(id, error, note)} />
            ) : (
                <NativeSelect id={id} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} className="text-sm" {...describedBy(id, error, note)}>
                    <option value="">{project.status === 'loading' ? 'Loading flows…' : emptyLabel}</option>
                    {value && !flow ? <option value={value}>{missing ? `${value} (not found)` : value}</option> : null}
                    {own.length > 0 ? <optgroup label={`${project.name}'s flows`}>{own.map(option)}</optgroup> : null}
                    {folders.map((group) => (
                        <optgroup key={group.label} label={project.own ? `Bundled with Spark: ${group.label}` : group.label}>{group.items.map(option)}</optgroup>
                    ))}
                </NativeSelect>
            )}
        </TriggerField>
    )
}

/** A count and a unit, kept in the form as entered; saved as seconds. */
function IntervalField({
    id,
    label,
    value,
    unit,
    onChange,
    error,
    hint,
    disabled,
}: {
    id: string
    label: string
    value: string
    unit: number
    onChange: (value: string, unit: number) => void
    error?: string
    hint?: string
    disabled: boolean
}) {
    return (
        <TriggerField label={label} htmlFor={id} hint={hint} error={error} className="text-sm">
            <div className="flex gap-2">
                <Input
                    id={id}
                    type="number"
                    min={1}
                    step="any"
                    inputMode="decimal"
                    value={value}
                    onChange={(event) => onChange(event.target.value, unit)}
                    disabled={disabled}
                    className="w-24 shrink-0 text-sm"
                    {...describedBy(id, error, hint)}
                />
                <NativeSelect aria-label={`${label} unit`} value={unit} onChange={(event) => onChange(value, Number(event.target.value))} disabled={disabled} className="text-sm">
                    {INTERVAL_UNITS.map((entry) => <option key={entry.seconds} value={entry.seconds}>{entry.label}</option>)}
                </NativeSelect>
            </div>
        </TriggerField>
    )
}

export function TriggerEditor({
    form,
    onChange,
    mode,
    protectedTrigger,
    errors,
}: {
    form: TriggerFormState
    onChange: (value: TriggerFormState) => void
    mode: 'create' | 'edit'
    protectedTrigger: boolean
    errors: TriggerFieldErrors
}) {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const [missions, setMissions] = useState<Mission[]>([])
    const [missionError, setMissionError] = useState('')
    useEffect(() => {
        let disposed = false
        setMissions([]); setMissionError('')
        if (form.actionMode === 'mission' && form.projectPath) {
            void request<Board>(form.projectPath).then(board => { if (!disposed) setMissions(board.missions.filter(m => !m.closed)) }).catch(error => { if (!disposed) setMissionError(error.message) })
        }
        return () => { disposed = true }
    }, [form.actionMode, form.projectPath])
    const summaries = useFlowSummaries(form.projectPath || null)
    const ownFlows = useOwnFlows(form.projectPath || null)
    const projectName = projectLabel(projectRegistry, form.projectPath)
    const projectFlows: ProjectFlows = !summaries ? { status: 'loading', flows: [], own: null, name: projectName }
        : summaries.error ? { status: 'error', flows: [], own: null, name: projectName }
        : { status: 'ready', flows: summaries.flows, own: ownFlows, name: projectName }
    // When a refused save first shows errors, move to the first field to fix.
    const rootRef = useRef<HTMLDivElement>(null)
    const hasErrors = Object.keys(errors).length > 0
    useEffect(() => {
        if (hasErrors) rootRef.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus()
    }, [hasErrors])
    const sourceTypeDisabled = protectedTrigger || mode === 'edit'
    const sourceConfigurationDisabled = protectedTrigger
    const fieldId = (key: keyof TriggerFormState) => `${mode}-trigger-${key}`
    const text = (key: keyof TriggerFormState, hint?: ReactNode) => ({
        id: fieldId(key),
        value: String(form[key]),
        onChange: (event: { target: { value: string } }) => onChange({ ...form, [key]: event.target.value }),
        ...describedBy(fieldId(key), errors[key], hint),
    })
    const field = (key: keyof TriggerFormState, label: string, hint?: ReactNode) => ({
        label,
        htmlFor: fieldId(key),
        hint,
        error: errors[key],
    })

    const daysId = fieldId('scheduleWeekdays')
    // Said, not converted: a saved trigger keeps the zone its time was set in.
    const timeHint = form.scheduleTimezone === browserTimeZone() ? 'In your time zone.' : `In ${form.scheduleTimezone}, not your time zone.`
    const itemsHint = 'Where the list is in the response, such as items or data.results.'
    const itemIdHint = 'What identifies each item, such as id. Each new one starts the action.'
    const headersHint = <>A JSON object, such as <code>{'{"Authorization": "Bearer …"}'}</code>.</>
    const statusesHint = 'completed, failed or validation_error, separated by commas. Leave empty for any.'
    // Editing: the status card shows the next run instead.
    const scheduleSeconds = intervalSeconds(form.scheduleInterval, form.scheduleIntervalUnit)
    const intervalHint = mode === 'create' && scheduleSeconds ? firstIntervalRunHint(scheduleSeconds) : undefined
    const contextHint = <>Values the flow can read when this trigger starts it, written as JSON, such as <code>{'{"branch": "main"}'}</code>.</>

    return (
        // One two-column grid for every field; selects fill their cell like inputs.
        <div ref={rootRef} className="grid gap-4 sm:grid-cols-2 [&_[data-slot=native-select-wrapper]]:w-full">
                <TriggerField {...field('name', 'Name')} className="text-sm">
                    <Input {...text('name')} className="text-sm" />
                </TriggerField>
                {/* A trigger belongs to a project: chosen on create, fixed after. */}
                <TriggerField label="Project" htmlFor={fieldId('projectPath')} className="text-sm">
                    <NativeSelect
                        id={fieldId('projectPath')}
                        value={form.projectPath}
                        onChange={(event) => onChange({ ...form, projectPath: event.target.value })}
                        disabled={mode === 'edit'}
                        className="text-sm"
                    >
                        {mode === 'edit' && !projectRegistry[form.projectPath] ? (
                            <option value={form.projectPath}>{form.projectPath}</option>
                        ) : null}
                        {orderProjects(projectRegistry).map((project) => (
                            <option key={project.directoryPath} value={project.directoryPath}>
                                {projectLabel(projectRegistry, project.directoryPath)}
                            </option>
                        ))}
                    </NativeSelect>
                </TriggerField>
                <TriggerField label="Action" htmlFor={fieldId('actionMode')}>
                    <NativeSelect id={fieldId('actionMode')} value={form.actionMode} disabled={protectedTrigger} onChange={e => onChange({ ...form, actionMode: e.target.value })}>
                        <option value="static">Start a flow</option><option value="mission">Deliver to a mission</option><option value="role">Deliver to a standing role</option>
                    </NativeSelect>
                </TriggerField>
                {form.actionMode === 'mission' ? <TriggerField label="Mission" htmlFor={fieldId('missionId')}>
                    <NativeSelect id={fieldId('missionId')} value={form.missionId} disabled={protectedTrigger} onChange={e => onChange({ ...form, missionId: e.target.value })}>
                        <option value="">Select an open mission</option>
                        {missions.map(m => <option key={m.id} value={m.id}>{m.fields.title}</option>)}
                    </NativeSelect>
                    {missionError && <p role="alert">{missionError}</p>}
                </TriggerField> : form.actionMode === 'role' ? <TriggerField {...field('roleName', 'Role')} className="text-sm">
                    <Input {...text('roleName')} disabled={protectedTrigger} placeholder="role-name" className="text-sm font-mono" />
                </TriggerField> : (
                    <FlowSelect
                        id={fieldId('flowName')}
                        label="Flow to start"
                        value={form.flowName}
                        onChange={(flowName) => onChange({ ...form, flowName })}
                        project={projectFlows}
                        emptyLabel="Choose a flow"
                        hint="Flows this project can run."
                        error={errors.flowName}
                    />
                )}
                <TriggerField label="Starts when" htmlFor={fieldId('sourceType')} className="text-sm">
                    <NativeSelect
                        id={fieldId('sourceType')}
                        value={form.sourceType}
                        onChange={(event) => onChange({ ...form, sourceType: event.target.value as TriggerSourceType })}
                        disabled={sourceTypeDisabled}
                        className="text-sm"
                    >
                        <option value="schedule">On a schedule</option>
                        <option value="poll">A web address has new items</option>
                        <option value="webhook">Another service calls a webhook</option>
                        <option value="flow_event">A flow run finishes</option>
                    </NativeSelect>
                </TriggerField>

            {form.sourceType === 'schedule' ? (
                <>
                    <TriggerField label="Repeats" htmlFor={fieldId('scheduleKind')} className="text-sm">
                        <NativeSelect
                            id={fieldId('scheduleKind')}
                            value={form.scheduleKind}
                            onChange={(event) => onChange({ ...form, scheduleKind: event.target.value as 'once' | 'interval' | 'weekly' })}
                            disabled={sourceConfigurationDisabled}
                            className="text-sm"
                        >
                            <option value="interval">At a set interval</option>
                            <option value="once">Once</option>
                            <option value="weekly">Weekly</option>
                        </NativeSelect>
                    </TriggerField>
                    {form.scheduleKind === 'interval' ? (
                        <IntervalField
                            id={fieldId('scheduleInterval')}
                            label="Every"
                            value={form.scheduleInterval}
                            unit={form.scheduleIntervalUnit}
                            onChange={(scheduleInterval, scheduleIntervalUnit) => onChange({ ...form, scheduleInterval, scheduleIntervalUnit })}
                            error={errors.scheduleInterval}
                            hint={intervalHint}
                            disabled={sourceConfigurationDisabled}
                        />
                    ) : null}
                    {form.scheduleKind === 'once' ? (
                        <TriggerField {...field('scheduleRunAt', 'Run at', 'In your time zone.')} className="text-sm">
                            <Input {...text('scheduleRunAt', 'In your time zone.')} type="datetime-local" disabled={sourceConfigurationDisabled} className="text-sm" />
                        </TriggerField>
                    ) : null}
                    {form.scheduleKind === 'weekly' ? (
                        <>
                            <Field className="content-start text-sm">
                                <FieldLabel id={`${daysId}-label`}>Days</FieldLabel>
                                {/* Focusable, so a refused save can move to it. */}
                                <SegmentedControl
                                    id={daysId}
                                    tabIndex={-1}
                                    aria-labelledby={`${daysId}-label`}
                                    {...describedBy(daysId, errors.scheduleWeekdays)}
                                    options={WEEKDAYS.map((day) => {
                                        const pressed = form.scheduleWeekdays.includes(day.key)
                                        // Kept in week order, as the server stores them.
                                        const next = WEEKDAYS.map((entry) => entry.key).filter((key) => (key === day.key ? !pressed : form.scheduleWeekdays.includes(key)))
                                        return {
                                            key: day.key,
                                            label: day.label,
                                            'aria-label': day.name,
                                            pressed,
                                            disabled: sourceConfigurationDisabled,
                                            onPress: () => onChange({ ...form, scheduleWeekdays: next }),
                                        }
                                    })}
                                />
                                {errors.scheduleWeekdays ? <FieldError id={`${daysId}-error`}>{errors.scheduleWeekdays}</FieldError> : null}
                            </Field>
                            <TriggerField {...field('scheduleTime', 'Time', timeHint)} className="text-sm">
                                <Input {...text('scheduleTime', timeHint)} type="time" disabled={sourceConfigurationDisabled} className="text-sm" />
                            </TriggerField>
                        </>
                    ) : null}
                </>
            ) : null}

            {form.sourceType === 'poll' ? (
                <>
                    <TriggerField {...field('pollUrl', 'Web address to check')} className="text-sm">
                        <Input {...text('pollUrl')} type="url" disabled={sourceConfigurationDisabled} className="text-sm font-mono" />
                    </TriggerField>
                    <IntervalField
                        id={fieldId('pollInterval')}
                        label="Check every"
                        value={form.pollInterval}
                        unit={form.pollIntervalUnit}
                        onChange={(pollInterval, pollIntervalUnit) => onChange({ ...form, pollInterval, pollIntervalUnit })}
                        error={errors.pollInterval}
                        disabled={sourceConfigurationDisabled}
                    />
                    <TriggerField {...field('pollItemsPath', 'List of items', itemsHint)} className="text-sm">
                        <Input {...text('pollItemsPath', itemsHint)} disabled={sourceConfigurationDisabled} className="text-sm font-mono" />
                    </TriggerField>
                    <TriggerField {...field('pollItemIdPath', 'Item ID', itemIdHint)} className="text-sm">
                        <Input {...text('pollItemIdPath', itemIdHint)} disabled={sourceConfigurationDisabled} className="text-sm font-mono" />
                    </TriggerField>
                    <TriggerField {...field('pollHeadersText', 'Request headers (optional)', headersHint)} className="text-sm sm:col-span-2">
                        <Textarea {...text('pollHeadersText', headersHint)} disabled={sourceConfigurationDisabled} className="min-h-20 font-mono text-sm" />
                    </TriggerField>
                </>
            ) : null}

            {form.sourceType === 'flow_event' ? (
                <>
                    <FlowSelect
                        id={fieldId('flowEventFlowName')}
                        label="Watched flow"
                        value={form.flowEventFlowName}
                        onChange={(flowEventFlowName) => onChange({ ...form, flowEventFlowName })}
                        project={projectFlows}
                        emptyLabel="Any flow"
                        hint="Runs of any flow in this project."
                        disabled={sourceConfigurationDisabled}
                    />
                    <TriggerField {...field('flowEventStatuses', 'When the run ends as', statusesHint)} className="text-sm">
                        <Input {...text('flowEventStatuses', statusesHint)} disabled={sourceConfigurationDisabled} className="text-sm" placeholder="completed,failed" />
                    </TriggerField>
                </>
            ) : null}

            {form.sourceType === 'webhook' ? (
                <Well className="p-3 text-sm text-muted-foreground sm:col-span-2">
                    Another service starts this trigger by sending a POST request to <code>{SHARED_WEBHOOK_ENDPOINT}</code>, with a key and secret that Spark creates for it.
                </Well>
            ) : null}

            {form.actionMode === 'static' && <TriggerField {...field('staticContextText', 'Extra data for the run (optional)', contextHint)} className="text-sm sm:col-span-2">
                <Textarea {...text('staticContextText', contextHint)} disabled={protectedTrigger} className="min-h-20 font-mono text-sm" />
            </TriggerField>}

            <div className="flex items-center gap-2 sm:col-span-2">
                <Switch
                    id={fieldId('enabled')}
                    checked={form.enabled}
                    onCheckedChange={(checked) => onChange({ ...form, enabled: checked })}
                />
                <Label htmlFor={fieldId('enabled')}>Enabled</Label>
            </div>
        </div>
    )
}
