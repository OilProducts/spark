import { Fragment, useState } from 'react'
import { NativeSelect } from '@/components/ui/native-select'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldError, FieldLabel } from '@/components/ui/field'
import { useAgentSettingsEditor } from './hooks/useAgentSettingsEditor'
import { SaveStatus } from './SaveStatus'
import { SettingsSummary } from './SettingsSummary'
import { agentLimitError, msToUnit, unitToMs, type agentLimitKeys, type DurationUnit } from './services/executionSettings'

const defaultPath = (key: string) => key === 'claude_config_dir' ? '<SPARK_HOME>/runtime/claude' : 'Platform default'
const labels = { codex_runtime_root: 'Codex runtime root', codex_seed_dir: 'Codex seed directory', claude_config_dir: 'Claude configuration directory', codex_jsonrpc_trace: 'Codex protocol log', agent_trace: 'Agent trace log' } as const
// [setting, label, what it affects, unit shown when the stored value is milliseconds]
const limitFields: [typeof agentLimitKeys[number], string, string, DurationUnit?][] = [
    ['max_turns', 'Maximum turns', 'Stops an agent session after this many turns. 0 means no limit.'],
    ['max_tool_rounds_per_input', 'Tool rounds per message', 'How many rounds of tool use an agent may take to answer one message. 0 means no limit.'],
    ['default_command_timeout_ms', 'Command time limit', 'How long a command may run when the agent sets no limit of its own.', 'seconds'],
    ['max_command_timeout_ms', 'Longest command time', 'No command may run longer than this, even when the agent asks for more.', 'minutes'],
    ['max_subagent_depth', 'Subagent levels', 'How many levels deep agents may start subagents. 0 turns subagents off.'],
    ['loop_detection_window', 'Loop check length', 'How many recent tool calls are compared to spot an agent repeating itself.'],
]
const permissionModes = { default: 'Default', acceptEdits: 'Accept edits', bypassPermissions: 'Bypass permissions', plan: 'Plan', dontAsk: 'Do not ask', auto: 'Automatic' } as const

export function AgentSettingsEditor() {
    const editor = useAgentSettingsEditor()
    const [newToolNames, setNewToolNames] = useState({ tool_output_limits: '', line_limits: '' })
    return <Card id="agent-session-limits" className="scroll-mt-4"><CardHeader><h2 className="text-base font-semibold">Agent session limits</h2></CardHeader><CardContent className="space-y-3">
        {editor.pending ? <p role="status">Updating…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
        {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error} className="text-sm text-destructive">{error}</p>)}
        {!editor.draft && editor.saved?.repair_defaults && <div className="space-y-1">
            <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Reset what can’t be read</Button>
            <p className="text-xs text-muted-foreground">Shows these settings with the ones that can’t be read set to their defaults, for you to check and save. ACP harnesses are kept as they are.</p>
        </div>}
        {editor.draft && <fieldset disabled={editor.pending} className="space-y-3">
            {limitFields.map(([key, label, help, unit]) => {
                const value = editor.draft![key]
                const error = agentLimitError(editor.draft!, key)
                return <Fragment key={key}>
                {key === 'loop_detection_window' && <Label className="flex items-center gap-2 text-sm"><Switch checked={editor.draft!.enable_loop_detection} onCheckedChange={(checked) => editor.setDraft((draft) => draft && ({ ...draft, enable_loop_detection: checked }))} />Warn agents stuck in a loop</Label>}
                <Field>
                <FieldLabel htmlFor={`agent-${key}`}>{label}</FieldLabel>
                <div className="flex items-center gap-2">
                    <Input id={`agent-${key}`} className="w-40" aria-invalid={!!error} aria-describedby={[unit && `agent-${key}-unit`, `agent-${key}-help`, error && `agent-${key}-error`].filter(Boolean).join(' ')} type="number" min={0} step={unit ? 'any' : 1}
                        value={unit ? msToUnit(value, unit) : Number.isFinite(value) ? value : ''}
                        onChange={(event) => { const text = event.target.value; editor.setDraft((draft) => draft && ({ ...draft, [key]: unit ? unitToMs(text, unit) : text === '' ? NaN : Number(text) })) }} />
                    {unit && <span id={`agent-${key}-unit`} className="text-sm text-muted-foreground">{unit}</span>}
                </div>
                <p id={`agent-${key}-help`} className="text-xs text-muted-foreground">{help}</p>
                {error && <FieldError id={`agent-${key}-error`} role="alert">{error}</FieldError>}
            </Field></Fragment>})}
            <details ref={(node) => { if (node && (Object.values(editor.draft?.native ?? {}).some((value) => typeof value === 'string' && !value.trim()))) node.open = true }}><SettingsSummary>Codex and Claude paths</SettingsSummary>
            <p className="text-xs text-muted-foreground">The Codex and Claude Code programs are set under Models & accounts › Agent harnesses.</p>
            {(['codex_runtime_root', 'codex_seed_dir', 'claude_config_dir'] as const).map((key) => <Field key={key}>
                <FieldLabel htmlFor={`agent-native-${key}`}>{labels[key]}</FieldLabel>
                <Input id={`agent-native-${key}`} aria-describedby={`agent-native-${key}-error`} aria-invalid={editor.draft?.native?.[key] != null && !editor.draft.native[key]?.trim()} value={editor.draft?.native?.[key] ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, [key]: event.target.value || null } }))} />
                {editor.draft?.native?.[key] != null && !editor.draft.native[key]?.trim() && <p id={`agent-native-${key}-error`} role="alert">Enter a nonempty path or leave blank for the default.</p>}
                {!editor.saved?.effective && editor.saved?.active_startup && ['codex_runtime_root', 'codex_seed_dir', 'claude_config_dir'].includes(key) && <p className="text-xs text-muted-foreground">Startup: {editor.saved.active_startup[key] ?? defaultPath(key)} · Retained until restart</p>}
                <p className="text-xs text-muted-foreground">In use: {editor.saved?.effective ? editor.saved.effective.native?.[key] ?? defaultPath(key) : 'Unavailable'} · {editor.saved?.sources?.[`native.${key}`]} · Requires restart</p>
            </Field>)}
            </details>
            <details><SettingsSummary>Permissions and environment</SettingsSummary>
            <Field><FieldLabel htmlFor="agent-environment-inheritance">Environment variables for tools</FieldLabel>
                <NativeSelect id="agent-environment-inheritance" value={editor.draft.environment_inheritance ?? 'inherit_core_only'} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, environment_inheritance: event.target.value as 'inherit_all' | 'inherit_none' | 'inherit_core_only' }))}>
                    <option value="inherit_core_only">Core variables only</option><option value="inherit_all">All variables</option><option value="inherit_none">None</option>
                </NativeSelect>
            </Field>
            <Field><FieldLabel htmlFor="agent-claude-permission">Claude permission mode</FieldLabel>
                <NativeSelect id="agent-claude-permission" value={editor.draft.native?.claude_permission_mode ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, claude_permission_mode: event.target.value || null } }))}>
                    <option value="">Default (bypass permissions)</option>
                    {Object.entries(permissionModes).map(([mode, name]) => <option key={mode} value={mode}>{name}</option>)}
                </NativeSelect>
                <p className="text-xs text-muted-foreground">In use: {editor.saved?.effective ? (() => { const mode = editor.saved.effective.native?.claude_permission_mode ?? 'bypassPermissions'; return permissionModes[mode as keyof typeof permissionModes] ?? mode })() : 'Unavailable'}</p>
            </Field>
            </details>
            <details><SettingsSummary>Debug logs</SettingsSummary>
            <p className="text-xs text-muted-foreground">Extra detail for troubleshooting. Leave off unless you are diagnosing a problem.</p>
            {(['codex_jsonrpc_trace', 'agent_trace'] as const).map((key) => <Field key={key}>
                <FieldLabel htmlFor={`agent-${key}`}>{labels[key]}</FieldLabel>
                <NativeSelect id={`agent-${key}`} value={editor.draft?.native?.[key] == null ? '' : String(editor.draft.native[key])} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, [key]: event.target.value === '' ? null : event.target.value === 'true' } }))}>
                    <option value="">Default (off)</option><option value="true">On</option><option value="false">Off</option>
                </NativeSelect>
                <p className="text-xs text-muted-foreground">In use: {editor.saved?.effective ? editor.saved.effective.native?.[key] ? 'On' : 'Off' : 'Unavailable'}</p>
            </Field>)}
            </details>
            <details ref={(node) => { if (node && ((['tool_output_limits', 'line_limits'] as const).some((key) => Object.entries(editor.draft![key]).some(([tool, value]) => !tool.trim() || !Number.isSafeInteger(value) || value < 0)))) node.open = true }}><SettingsSummary>Tool output limits</SettingsSummary>
            <p className="text-xs text-muted-foreground">Cut long tool output short before an agent reads it. Add a limit by tool name, such as shell.</p>
            {(['tool_output_limits', 'line_limits'] as const).map((key) => <fieldset key={key} className="space-y-2">
                <legend>{key === 'tool_output_limits' ? 'Tool output character limits' : 'Tool output line limits'}</legend>
                {Object.entries(editor.draft![key]).map(([tool, limit]) => <Field key={tool}>
                    <FieldLabel htmlFor={`agent-${key}-${tool}`}>{tool}</FieldLabel>
                    <Input id={`agent-${key}-${tool}`} className="w-40" aria-describedby={`agent-${key}-${tool}-error`} type="number" min={0} step={1} value={Number.isFinite(limit) ? limit : ''} aria-invalid={!Number.isSafeInteger(limit) || limit < 0} onChange={(event) => { const text = event.target.value; editor.setDraft((draft) => draft && ({ ...draft, [key]: { ...draft[key], [tool]: text === '' ? NaN : Number(text) } })) }} />
                    {(!Number.isSafeInteger(limit) || limit < 0) && <FieldError id={`agent-${key}-${tool}-error`} role="alert">Enter a whole number, 0 or more.</FieldError>}
                    <Button variant="outline" onClick={() => editor.setDraft((draft) => { if (!draft) return draft; const limits = { ...draft[key] }; delete limits[tool]; return { ...draft, [key]: limits } })}>Remove {tool} {key === 'line_limits' ? 'line' : 'character'} limit</Button>
                </Field>)}
                <FieldLabel htmlFor={`agent-new-${key}`}>Tool name for {key === 'line_limits' ? 'line' : 'character'} limit</FieldLabel>
                <Input id={`agent-new-${key}`} value={newToolNames[key]} onChange={(event) => setNewToolNames((names) => ({ ...names, [key]: event.target.value }))} />
                <Button variant="outline" disabled={!newToolNames[key].trim()} onClick={() => { const tool = newToolNames[key].trim(); editor.setDraft((draft) => draft && ({ ...draft, [key]: { ...draft[key], [tool]: draft[key][tool] ?? 1000 } })); setNewToolNames((names) => ({ ...names, [key]: '' })) }}>Add {key === 'line_limits' ? 'line' : 'character'} limit</Button>
            </fieldset>)}
            </details>
        </fieldset>}
        <div className="flex flex-wrap gap-2">
        <Button size="sm" aria-label="Save agent session limits" disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save</Button>
        <Button size="sm" aria-label="Discard agent session limits changes" variant="ghost" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
        </div>
        <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
    </CardContent></Card>
}
