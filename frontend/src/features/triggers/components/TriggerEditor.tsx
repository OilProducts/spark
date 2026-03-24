import type { TriggerSourceType } from '@/lib/workspaceClient'
import { SHARED_WEBHOOK_ENDPOINT, type TriggerFormState } from '../model/triggerForm'

export function TriggerEditor({
    form,
    onChange,
    mode,
    protectedTrigger,
}: {
    form: TriggerFormState
    onChange: (value: TriggerFormState) => void
    mode: 'create' | 'edit'
    protectedTrigger: boolean
}) {
    const sourceTypeDisabled = protectedTrigger || mode === 'edit'

    return (
        <div className="mt-4 space-y-3">
            <div className="grid gap-3 lg:grid-cols-2">
                <label className="space-y-1 text-sm">
                    <span className="text-xs font-medium text-foreground">Name</span>
                    <input
                        value={form.name}
                        onChange={(event) => onChange({ ...form, name: event.target.value })}
                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                    />
                </label>
                <label className="space-y-1 text-sm">
                    <span className="text-xs font-medium text-foreground">Target Flow</span>
                    <input
                        value={form.flowName}
                        onChange={(event) => onChange({ ...form, flowName: event.target.value })}
                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm font-mono"
                    />
                </label>
            </div>

            <div className="grid gap-3 lg:grid-cols-3">
                <label className="space-y-1 text-sm">
                    <span className="text-xs font-medium text-foreground">Source Type</span>
                    <select
                        value={form.sourceType}
                        onChange={(event) => onChange({ ...form, sourceType: event.target.value as TriggerSourceType })}
                        disabled={sourceTypeDisabled}
                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm disabled:cursor-not-allowed disabled:opacity-60"
                    >
                        <option value="schedule">Schedule</option>
                        <option value="poll">Poll</option>
                        <option value="webhook">Webhook</option>
                        <option value="flow_event">Flow Event</option>
                        {protectedTrigger ? <option value="workspace_event">Workspace Event</option> : null}
                    </select>
                </label>
                <label className="space-y-1 text-sm">
                    <span className="text-xs font-medium text-foreground">Project Target</span>
                    <input
                        value={form.projectPath}
                        onChange={(event) => onChange({ ...form, projectPath: event.target.value })}
                        disabled={protectedTrigger}
                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm disabled:cursor-not-allowed disabled:opacity-60"
                    />
                </label>
                <label className="flex items-end gap-2 text-sm">
                    <input
                        type="checkbox"
                        checked={form.enabled}
                        onChange={(event) => onChange({ ...form, enabled: event.target.checked })}
                        className="h-4 w-4"
                    />
                    <span className="text-xs font-medium text-foreground">Enabled</span>
                </label>
            </div>

            {form.sourceType === 'schedule' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Schedule Kind</span>
                        <select
                            value={form.scheduleKind}
                            onChange={(event) => onChange({ ...form, scheduleKind: event.target.value as 'once' | 'interval' | 'weekly' })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                        >
                            <option value="interval">Interval</option>
                            <option value="once">One Shot</option>
                            <option value="weekly">Weekly</option>
                        </select>
                    </label>
                    {form.scheduleKind === 'interval' ? (
                        <label className="space-y-1 text-sm">
                            <span className="text-xs font-medium text-foreground">Interval Seconds</span>
                            <input
                                value={form.scheduleIntervalSeconds}
                                onChange={(event) => onChange({ ...form, scheduleIntervalSeconds: event.target.value })}
                                className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                            />
                        </label>
                    ) : null}
                    {form.scheduleKind === 'once' ? (
                        <label className="space-y-1 text-sm lg:col-span-2">
                            <span className="text-xs font-medium text-foreground">Run At (ISO UTC)</span>
                            <input
                                value={form.scheduleRunAt}
                                onChange={(event) => onChange({ ...form, scheduleRunAt: event.target.value })}
                                className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm font-mono"
                                placeholder="2026-03-22T15:00:00Z"
                            />
                        </label>
                    ) : null}
                    {form.scheduleKind === 'weekly' ? (
                        <>
                            <label className="space-y-1 text-sm">
                                <span className="text-xs font-medium text-foreground">Weekdays</span>
                                <input
                                    value={form.scheduleWeekdays}
                                    onChange={(event) => onChange({ ...form, scheduleWeekdays: event.target.value })}
                                    className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                                    placeholder="mon,wed,fri"
                                />
                            </label>
                            <div className="grid grid-cols-2 gap-3">
                                <label className="space-y-1 text-sm">
                                    <span className="text-xs font-medium text-foreground">Hour</span>
                                    <input
                                        value={form.scheduleHour}
                                        onChange={(event) => onChange({ ...form, scheduleHour: event.target.value })}
                                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                                    />
                                </label>
                                <label className="space-y-1 text-sm">
                                    <span className="text-xs font-medium text-foreground">Minute</span>
                                    <input
                                        value={form.scheduleMinute}
                                        onChange={(event) => onChange({ ...form, scheduleMinute: event.target.value })}
                                        className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                                    />
                                </label>
                            </div>
                        </>
                    ) : null}
                </div>
            ) : null}

            {form.sourceType === 'poll' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <label className="space-y-1 text-sm lg:col-span-2">
                        <span className="text-xs font-medium text-foreground">Poll URL</span>
                        <input
                            value={form.pollUrl}
                            onChange={(event) => onChange({ ...form, pollUrl: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm font-mono"
                        />
                    </label>
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Interval Seconds</span>
                        <input
                            value={form.pollIntervalSeconds}
                            onChange={(event) => onChange({ ...form, pollIntervalSeconds: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                        />
                    </label>
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Items Path</span>
                        <input
                            value={form.pollItemsPath}
                            onChange={(event) => onChange({ ...form, pollItemsPath: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                        />
                    </label>
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Item ID Path</span>
                        <input
                            value={form.pollItemIdPath}
                            onChange={(event) => onChange({ ...form, pollItemIdPath: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                        />
                    </label>
                    <label className="space-y-1 text-sm lg:col-span-2">
                        <span className="text-xs font-medium text-foreground">Headers JSON</span>
                        <textarea
                            value={form.pollHeadersText}
                            onChange={(event) => onChange({ ...form, pollHeadersText: event.target.value })}
                            className="min-h-24 w-full rounded-md border border-input bg-background px-2 py-2 font-mono text-xs"
                        />
                    </label>
                </div>
            ) : null}

            {form.sourceType === 'flow_event' ? (
                <div className="grid gap-3 lg:grid-cols-2">
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Observed Flow</span>
                        <input
                            value={form.flowEventFlowName}
                            onChange={(event) => onChange({ ...form, flowEventFlowName: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm font-mono"
                            placeholder="Leave blank for any observed flow"
                        />
                    </label>
                    <label className="space-y-1 text-sm">
                        <span className="text-xs font-medium text-foreground">Terminal Statuses</span>
                        <input
                            value={form.flowEventStatuses}
                            onChange={(event) => onChange({ ...form, flowEventStatuses: event.target.value })}
                            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                            placeholder="completed,failed"
                        />
                    </label>
                </div>
            ) : null}

            {form.sourceType === 'webhook' ? (
                <div className="rounded-md border border-border bg-background/70 px-3 py-2 text-sm text-muted-foreground">
                    Webhook triggers use the shared ingress endpoint at <code>{SHARED_WEBHOOK_ENDPOINT}</code>. The key and secret are generated automatically.
                </div>
            ) : null}

            <label className="space-y-1 text-sm">
                <span className="text-xs font-medium text-foreground">Static Context JSON</span>
                <textarea
                    value={form.staticContextText}
                    onChange={(event) => onChange({ ...form, staticContextText: event.target.value })}
                    disabled={protectedTrigger}
                    className="min-h-24 w-full rounded-md border border-input bg-background px-2 py-2 font-mono text-xs disabled:cursor-not-allowed disabled:opacity-60"
                />
            </label>
        </div>
    )
}
