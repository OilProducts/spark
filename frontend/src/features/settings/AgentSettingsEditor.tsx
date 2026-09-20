import { useState } from 'react'
import { NativeSelect } from '@/components/ui/native-select'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useAgentSettingsEditor } from './hooks/useAgentSettingsEditor'

const labels = { max_turns: 'Maximum turns', max_tool_rounds_per_input: 'Maximum tool rounds per input', default_command_timeout_ms: 'Default command timeout (milliseconds)', max_command_timeout_ms: 'Maximum command timeout (milliseconds)', loop_detection_window: 'Loop detection window', max_subagent_depth: 'Maximum subagent depth', codex_binary: 'Codex binary', codex_runtime_root: 'Codex runtime root', codex_seed_dir: 'Codex seed directory', claude_binary: 'Claude binary', claude_config_dir: 'Claude configuration directory', codex_jsonrpc_trace: 'Codex JSON-RPC tracing', agent_trace: 'Agent tracing' } as const

export function AgentSettingsEditor() {
    const editor = useAgentSettingsEditor()
    const [newToolNames, setNewToolNames] = useState({ tool_output_limits: '', line_limits: '' })
    return <Card><CardHeader><h3 className="text-sm font-semibold">Agent session limits</h3></CardHeader><CardContent className="space-y-3">
        {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
        <p className="text-xs">Settings apply to new work. Agent homes and seed locations require restart. Zero turn limits mean unlimited. Codex retains its standard service tier, never-ask approval policy, and full-access sandbox policy.</p>
        {editor.draft && <fieldset disabled={editor.pending} className="space-y-2">
            {(['max_turns', 'max_tool_rounds_per_input', 'default_command_timeout_ms', 'max_command_timeout_ms', 'loop_detection_window', 'max_subagent_depth'] as const).map((key) => {
                const value = editor.draft![key]
                const invalid = !Number.isSafeInteger(value) || value < 0 || (key === 'default_command_timeout_ms' && value <= 0) || (key === 'max_command_timeout_ms' && value < editor.draft!.default_command_timeout_ms) || (key === 'loop_detection_window' && editor.draft!.enable_loop_detection && value <= 0)
                return <Field key={key}>
                <FieldLabel htmlFor={`agent-${key}`}>{labels[key]}</FieldLabel>
                <Input id={`agent-${key}`} aria-invalid={invalid} aria-describedby={invalid ? "agent-limits-error" : undefined} type="number" min={0} step={1} value={editor.draft![key]} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, [key]: Number(event.target.value) }))} />
            </Field>})}
            <label><input type="checkbox" checked={editor.draft.enable_loop_detection} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, enable_loop_detection: event.target.checked }))} /> Enable loop detection</label>
            <details ref={(node) => { if (node && (Object.values(editor.draft?.native ?? {}).some((value) => typeof value === 'string' && !value.trim()))) node.open = true }}><summary>Runtime paths</summary>
            {(['codex_binary', 'codex_runtime_root', 'codex_seed_dir', 'claude_binary', 'claude_config_dir'] as const).map((key) => <Field key={key}>
                <FieldLabel htmlFor={`agent-native-${key}`}>{labels[key]}</FieldLabel>
                <Input id={`agent-native-${key}`} aria-describedby={`agent-native-${key}-error`} aria-invalid={editor.draft?.native?.[key] != null && !editor.draft.native[key]?.trim()} value={editor.draft?.native?.[key] ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, [key]: event.target.value || null } }))} />
                {editor.draft?.native?.[key] != null && !editor.draft.native[key]?.trim() && <p id={`agent-native-${key}-error`} role="alert">Enter a nonempty path or leave blank for the default.</p>}
                {!editor.saved?.effective && editor.saved?.active_startup && ['codex_runtime_root', 'codex_seed_dir', 'claude_config_dir'].includes(key) && <p className="text-xs">Startup: {editor.saved.active_startup[key] ?? 'Platform default'} · Retained until restart</p>}
                <p className="text-xs">Effective: {editor.saved?.effective ? editor.saved.effective.native?.[key] ?? 'Platform default' : 'Unavailable'} · {editor.saved?.sources?.[`native.${key}`]}{key.endsWith('_binary') ? '' : ' · Requires restart'}</p>
            </Field>)}
            </details>
            <details><summary>Permissions and environment</summary>
            <Field><FieldLabel htmlFor="agent-environment-inheritance">Tool environment inheritance</FieldLabel>
                <NativeSelect id="agent-environment-inheritance" value={editor.draft.environment_inheritance ?? 'inherit_core_only'} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, environment_inheritance: event.target.value as 'inherit_all' | 'inherit_none' | 'inherit_core_only' }))}>
                    <option value="inherit_core_only">Core variables only</option><option value="inherit_all">All variables</option><option value="inherit_none">None</option>
                </NativeSelect>
            </Field>
            <Field><FieldLabel htmlFor="agent-claude-permission">Claude permission mode</FieldLabel>
                <NativeSelect id="agent-claude-permission" value={editor.draft.native?.claude_permission_mode ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, claude_permission_mode: event.target.value || null } }))}>
                    <option value="">Default (bypass permissions)</option>
                    {(['default', 'acceptEdits', 'bypassPermissions', 'plan', 'dontAsk', 'auto'] as const).map((mode) => <option key={mode} value={mode}>{{ default: 'Default', acceptEdits: 'Accept edits', bypassPermissions: 'Bypass permissions', plan: 'Plan', dontAsk: 'Do not ask', auto: 'Automatic' }[mode]}</option>)}
                </NativeSelect>
                <p className="text-xs">Effective: {editor.saved?.effective ? editor.saved.effective.native?.claude_permission_mode ?? 'bypassPermissions' : 'Unavailable'}</p>
            </Field>
            </details>
            <details><summary>Tracing</summary>
            {(['codex_jsonrpc_trace', 'agent_trace'] as const).map((key) => <Field key={key}>
                <FieldLabel htmlFor={`agent-${key}`}>{labels[key]}</FieldLabel>
                <NativeSelect id={`agent-${key}`} value={editor.draft?.native?.[key] == null ? '' : String(editor.draft.native[key])} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, native: { ...draft.native, [key]: event.target.value === '' ? null : event.target.value === 'true' } }))}>
                    <option value="">Default (off)</option><option value="true">On</option><option value="false">Off</option>
                </NativeSelect>
                <p className="text-xs">Effective: {editor.saved?.effective ? editor.saved.effective.native?.[key] ? 'On' : 'Off' : 'Unavailable'}</p>
            </Field>)}
            </details>
            <details ref={(node) => { if (node && ((['tool_output_limits', 'line_limits'] as const).some((key) => Object.entries(editor.draft![key]).some(([tool, value]) => !tool.trim() || !Number.isSafeInteger(value) || value < 0)))) node.open = true }}><summary>Tool-output limits</summary>
            {(['tool_output_limits', 'line_limits'] as const).map((key) => <fieldset key={key} className="space-y-2">
                <legend>{key === 'tool_output_limits' ? 'Tool output character limits' : 'Tool output line limits'}</legend>
                {Object.entries(editor.draft![key]).map(([tool, limit]) => <Field key={tool}>
                    <FieldLabel htmlFor={`agent-${key}-${tool}`}>{tool}</FieldLabel>
                    <Input id={`agent-${key}-${tool}`} aria-describedby={editor.invalid ? "agent-limits-error" : undefined} type="number" min={0} step={1} value={limit} aria-invalid={!Number.isSafeInteger(limit) || limit < 0} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, [key]: { ...draft[key], [tool]: Number(event.target.value) } }))} />
                    <Button variant="outline" onClick={() => editor.setDraft((draft) => { if (!draft) return draft; const limits = { ...draft[key] }; delete limits[tool]; return { ...draft, [key]: limits } })}>Remove {tool} {key === 'line_limits' ? 'line' : 'character'} limit</Button>
                </Field>)}
                <FieldLabel htmlFor={`agent-new-${key}`}>Tool name for {key === 'line_limits' ? 'line' : 'character'} limit</FieldLabel>
                <Input id={`agent-new-${key}`} value={newToolNames[key]} onChange={(event) => setNewToolNames((names) => ({ ...names, [key]: event.target.value }))} />
                <Button variant="outline" disabled={!newToolNames[key].trim()} onClick={() => { const tool = newToolNames[key].trim(); editor.setDraft((draft) => draft && ({ ...draft, [key]: { ...draft[key], [tool]: draft[key][tool] ?? 1000 } })); setNewToolNames((names) => ({ ...names, [key]: '' })) }}>Add {key === 'line_limits' ? 'line' : 'character'} limit</Button>
            </fieldset>)}
            </details>
            {editor.invalid && <p id="agent-limits-error" role="alert">Use nonnegative whole numbers, a positive default timeout no greater than the maximum, and a positive window when loop detection is enabled.</p>}
        </fieldset>}
        <div className="flex flex-wrap gap-2">
        <Button disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save agent settings</Button>
        <Button variant="outline" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard agent changes</Button>
        </div>
        {editor.error && <p role="alert">{editor.error}</p>}{editor.message && <p role="status">{editor.message}</p>}
    </CardContent></Card>
}
