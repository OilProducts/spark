import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { Well } from '@/components/app/well'
import { StatusPill, type StatusTone } from '@/components/app/status'
import { useDialogController } from '@/components/app/dialog-controller'
import { CodexConnectionControls } from './CodexConnectionSettings'
import { ClaudeCodeConnectionControls } from './ClaudeCodeConnectionSettings'
import { SettingsSummary } from './SettingsSummary'
import { SaveStatus } from './SaveStatus'
import { useSettingsNavigationProtection } from './hooks/useSettingsNavigationProtection'
import { fetchAgentSettings, fetchHarnessCatalog, HIDDEN_ENV_VALUE, saveAcpHarness, saveAgentSettings, type AcpHarnessEntry, type AcpHarnessRegistration, type AgentSettingsView,
    type HarnessSetupStatus, type ProjectChatModelCapability, type ProjectChatModelProviderStatusResponse, type ProjectChatModelsResponse } from './services/executionSettings'

type Connection = 'connected' | 'disconnected' | 'pending' | 'error'
type Status = HarnessSetupStatus | 'signed_out' | 'signing_in' | 'checking'
type Provider = ProjectChatModelProviderStatusResponse | undefined

const statuses: Record<Status, [string, StatusTone]> = {
    ready: ['Ready', 'success'],
    checking: ['Checking…', 'muted'],
    signing_in: ['Signing in…', 'info'],
    signed_out: ['Signed out', 'warning'],
    unauthenticated: ['Signed out', 'warning'],
    not_installed: ['Not installed', 'danger'],
    unreachable: ['Not responding', 'danger'],
    misconfigured: ['Misconfigured', 'danger'],
}

const capabilityNames: Record<string, string> = {
    model_selection: 'Model choice', reasoning_effort: 'Reasoning effort', session_resume: 'Resuming sessions',
    pinned_instructions: 'Pinned instructions', plan_mode: 'Plan mode', steering: 'Messages during a turn',
    child_intervention: 'Subagent control', forms: 'Questions and forms', per_prompt_usage: 'Token usage',
}
const stateWords = { supported: 'Yes', unsupported: 'No', unverified: 'Unverified' } as const
// Spark sets these for every agent process.
const reservedEnv = ['SPARK_HOME', 'SPARK_API_BASE_URL', 'SPARK_PROJECT_WORK_DIR']

const errorText = (error: unknown, fallback: string) => error instanceof Error ? error.message : fallback

/** Every harness that can run chats and flows, whether it is ready, and how to fix it when it isn't. */
export function HarnessSettings() {
    const [view, setView] = useState<AgentSettingsView | null>(null)
    const [viewError, setViewError] = useState('')
    const [catalog, setCatalog] = useState<ProjectChatModelsResponse | null>(null)
    const [catalogError, setCatalogError] = useState('')
    const [checking, setChecking] = useState(false)
    // Remounts the sign-in controls, which read their state when mounted.
    const [checks, setChecks] = useState(0)
    const [connections, setConnections] = useState<Record<string, Connection>>({})
    const [editing, setEditing] = useState<string | null>(null)
    const [message, setMessage] = useState('')
    const [error, setError] = useState('')
    const { confirm } = useDialogController()
    const revision = useRef<string | null>(null)
    const request = useRef(0)

    const show = useCallback((next: AgentSettingsView) => { revision.current = next.revision; setView(next); setViewError('') }, [])
    const load = useCallback(() => fetchAgentSettings().then(show, (error: unknown) => setViewError(errorText(error, 'Unable to load harness settings.'))), [show])
    const check = useCallback((refresh: boolean) => {
        const current = ++request.current
        setChecking(true)
        return fetchHarnessCatalog(null, refresh).then((next) => {
            if (current === request.current) { setCatalog(next); setCatalogError('') }
        }, (error: unknown) => {
            if (current === request.current) setCatalogError(errorText(error, 'Unable to check harnesses.'))
        }).finally(() => { if (current === request.current) setChecking(false) })
    }, [])
    const recheck = useCallback(() => { setChecks((value) => value + 1); void check(true) }, [check])
    const saved = useCallback((next: AgentSettingsView, text: string) => { show(next); setMessage(text); setError(''); void check(false) }, [show, check])
    const onConnection = useCallback((id: string, state: Connection) => setConnections((current) => current[id] === state ? current : { ...current, [id]: state }), [])

    useEffect(() => {
        void load()
        void check(false)
        const refresh = (event: Event) => {
            const payload = (event as CustomEvent<{ payload?: { section?: string; revision?: unknown } }>).detail?.payload
            if (!payload?.section || !['agents', 'codex', 'claude_code'].includes(payload.section)) return
            // This card's own saves already show their result.
            if (payload.section === 'agents' && payload.revision === revision.current) return
            void load()
            void check(false)
        }
        window.addEventListener('spark:settings-live-event', refresh)
        return () => window.removeEventListener('spark:settings-live-event', refresh)
    }, [load, check])

    const remove = async (id: string, name: string) => {
        if (!view || !await confirm({ title: `Remove ${name}?`, description: `Chats and flows that use acp:${id} will stop working until it is added again.`, confirmLabel: 'Remove', cancelLabel: 'Cancel', confirmVariant: 'destructive' })) return
        setError('')
        try { saved(await saveAcpHarness(view.revision, id, null), `Removed ${name}.`) } catch (error) { setError(errorText(error, `Unable to remove ${name}.`)) }
    }

    const providers: Partial<ProjectChatModelsResponse['providers']> = catalog?.providers ?? {}
    const acp = Object.entries(view?.acp_harnesses ?? {})
    return <Card className="gap-4 py-4">
        <CardHeader className="gap-1 px-4"><h2 className="text-base font-semibold">Agent harnesses</h2></CardHeader>
        <CardContent className="space-y-4 px-4 pt-0">
            <p className="text-sm text-muted-foreground">The agents that run your chats and flows. Codex and Claude Code are built in; add any agent that speaks the Agent Client Protocol (ACP).</p>
            {viewError && <p role="alert" className="text-sm text-destructive">{viewError}</p>}
            {view && !view.stored && <p role="alert" className="text-sm text-destructive">Agent settings can’t be read: {view.validation_errors?.join(' ')} Fix them under Execution › Agent session limits to change programs here.</p>}
            {catalogError && <p role="alert" className="text-sm text-destructive">Unable to check harnesses: {catalogError}</p>}
            {(['codex', 'claude-code'] as const).map((id) => <NativeHarness key={id} id={id} view={view} provider={providers[id]}
                connection={connections[id]} onConnection={onConnection} checking={checking} checks={checks} onCheck={recheck} onSaved={saved} />)}
            {acp.map(([id, entry]) => editing === id
                ? <AcpHarnessForm key={id} id={id} entry={entry} view={view} onCancel={() => setEditing(null)} onSaved={(next, name) => { setEditing(null); saved(next, `Saved ${name}. Checking it now…`) }} />
                : <AcpHarness key={id} id={id} entry={entry} provider={providers[`acp:${id}`]} checking={checking} onCheck={recheck}
                    onEdit={() => { setEditing(id); setMessage('') }} onRemove={(name) => void remove(id, name)} />)}
            {editing === '' ? <AcpHarnessForm id={null} entry={null} view={view} onCancel={() => setEditing(null)} onSaved={(next, name) => { setEditing(null); saved(next, `Added ${name}. Checking it now…`) }} />
                : <Button variant="outline" disabled={!view} onClick={() => { setEditing(''); setMessage('') }}>Add ACP harness</Button>}
            <SaveStatus message={message} error={error} />
        </CardContent>
    </Card>
}

const natives = {
    codex: { name: 'Codex', key: 'codex_binary', program: 'codex', install: 'Install Codex' },
    'claude-code': { name: 'Claude Code', key: 'claude_binary', program: 'claude', install: 'Install Claude Code' },
} as const

function nativeStatus(provider: Provider, connection: Connection | undefined, checking: boolean): Status {
    if (checking || !provider) return 'checking'
    if (provider.setup_status === 'not_installed') return 'not_installed'
    if (connection === 'pending') return 'signing_in'
    if (connection === 'disconnected') return 'signed_out'
    if (provider.status === 'unavailable' || connection === 'error') return 'unreachable'
    return connection === 'connected' ? 'ready' : 'checking'
}

function NativeHarness({ id, view, provider, connection, onConnection, checking, checks, onCheck, onSaved }: {
    id: keyof typeof natives; view: AgentSettingsView | null; provider: Provider; connection?: Connection; onConnection: (id: string, state: Connection) => void
    checking: boolean; checks: number; onCheck: () => void; onSaved: (view: AgentSettingsView, message: string) => void
}) {
    const { name, key, program, install } = natives[id]
    const status = nativeStatus(provider, connection, checking)
    const stored = view?.stored?.native?.[key] ?? ''
    const [draft, setDraft] = useState<string | null>(null)
    const value = draft ?? stored
    const dirty = draft !== null && draft.trim() !== stored
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    useSettingsNavigationProtection(dirty, pending, `${name} program`)
    const onStatus = useCallback((state: Connection) => onConnection(id, state), [id, onConnection])
    const save = async () => {
        if (!view?.stored) return
        setPending(true); setError('')
        try {
            const next = await saveAgentSettings(view.revision, { ...view.stored, native: { ...view.stored.native, [key]: value.trim() || null } })
            setDraft(null)
            onSaved(next, `Saved the ${name} program. Checking it now…`)
        } catch (error) { setError(errorText(error, `Unable to save the ${name} program.`)) } finally { setPending(false) }
    }
    const next = status === 'not_installed' ? `${install}, or set Program below to the full path of its ${program} executable, then check again.`
        : status === 'signed_out' ? `Sign in below, then check again.`
            : status === 'unreachable' ? `Check that Program below is ${name}'s ${program} executable and that it starts from a terminal, then check again.` : ''
    const ready = status === 'ready'
    const builtIn = { state: 'supported', reason: 'Built in.' } as const
    return <HarnessRow name={name} detail={<>Built in · runs <code>{view?.effective?.native?.[key] ?? program}</code></>} status={status}
        problem={status === 'signed_out' ? `${name} is not signed in.` : status === 'not_installed' || status === 'unreachable' ? provider?.error ?? null : null} next={next}
        chat={ready ? builtIn : undefined} flow={ready ? builtIn : undefined}>
        <Field>
            <FieldLabel htmlFor={`harness-${id}-program`}>Program</FieldLabel>
            <div className="flex flex-wrap gap-2">
                <Input id={`harness-${id}-program`} className="min-w-0 flex-1" value={value} placeholder={`${program} (found on PATH)`} disabled={pending || !view?.stored}
                    aria-describedby={`harness-${id}-program-help`} onChange={(event) => setDraft(event.target.value)} />
                <Button size="sm" aria-label={`Save ${name} program`} disabled={!dirty || pending || !view?.stored} onClick={() => void save()}>Save</Button>
            </div>
            <p id={`harness-${id}-program-help`} className="text-xs text-muted-foreground">
                In use: {view?.effective?.native?.[key] ?? `${program} from PATH`}{view?.sources?.[`native.${key}`] ? ` · ${view.sources[`native.${key}`]}` : ''}. Leave blank to find {program} on PATH.
            </p>
            {error && <FieldError role="alert">{error}</FieldError>}
        </Field>
        {status !== 'not_installed' && (id === 'codex'
            ? <CodexConnectionControls key={checks} onStatus={onStatus} />
            : <ClaudeCodeConnectionControls key={checks} onStatus={onStatus} />)}
        <Button size="sm" variant="outline" aria-label={`Check ${name} again`} disabled={checking} onClick={onCheck}>Check again</Button>
    </HarnessRow>
}

function acpNextStep(status: Status): string {
    switch (status) {
        case 'misconfigured': return 'Edit the registration to fix it, or remove it.'
        case 'not_installed': return 'Install the agent, or edit Program to the full path of its executable, then check again.'
        case 'unauthenticated': return 'Sign in with the agent’s own sign-in, or edit its environment to give it the credentials it reads (such as an API key), then check again.'
        case 'unreachable': return 'Check its program, arguments and environment against the agent’s instructions for ACP, then check again.'
        default: return ''
    }
}

function AcpHarness({ id, entry, provider, checking, onCheck, onEdit, onRemove }: {
    id: string; entry: AcpHarnessEntry; provider: Provider; checking: boolean; onCheck: () => void; onEdit: () => void; onRemove: (name: string) => void
}) {
    const registration = entry.registration
    const name = typeof registration.name === 'string' && registration.name.trim() ? registration.name : id
    const status: Status = entry.error ? 'misconfigured' : checking || !provider?.setup_status ? 'checking' : provider.setup_status
    const command = [registration.executable, ...(Array.isArray(registration.args) ? registration.args : [])].filter((part) => typeof part === 'string').join(' ')
    const env = Object.keys(registration.env && typeof registration.env === 'object' ? registration.env : {})
    const capabilities = Object.entries(provider?.capabilities ?? {})
    return <HarnessRow name={name} detail={<>ACP · <code>acp:{id}</code>{command && <> · runs <code>{command}</code></>}</>} status={status}
        problem={entry.error ?? (status === 'checking' || status === 'ready' ? null : provider?.error || null)} next={acpNextStep(status)}
        chat={status === 'ready' ? provider?.chat : undefined} flow={status === 'ready' ? provider?.flow : undefined}>
        {env.length > 0 && <p className="text-xs text-muted-foreground">Environment: {env.join(', ')} (values hidden)</p>}
        {status === 'ready' && capabilities.length > 0 && <details>
            <SettingsSummary>What Spark can use</SettingsSummary>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 pt-1 text-sm">
                {capabilities.map(([key, capability]) => <CapabilityRow key={key} name={capabilityNames[key] ?? key} capability={capability} />)}
            </dl>
        </details>}
        <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" aria-label={`Edit ${name}`} onClick={onEdit}>Edit</Button>
            <Button size="sm" variant="outline" aria-label={`Check ${name} again`} disabled={checking} onClick={onCheck}>Check again</Button>
            <Button size="sm" variant="ghost" aria-label={`Remove ${name}`} onClick={() => onRemove(name)}>Remove</Button>
        </div>
    </HarnessRow>
}

function CapabilityRow({ name, capability }: { name: string; capability?: ProjectChatModelCapability }) {
    return <>
        <dt className="text-muted-foreground">{name}</dt>
        <dd>{capability ? <>{stateWords[capability.state]}{capability.state === 'supported' && capability.reason === 'Built in.' ? '' : <span className="text-muted-foreground"> — {capability.reason}</span>}</> : <span className="text-muted-foreground">Not until it is ready</span>}</dd>
    </>
}

function HarnessRow({ name, detail, status, problem, next, chat, flow, children }: {
    name: string; detail: ReactNode; status: Status; problem: string | null; next: string; chat?: ProjectChatModelCapability; flow?: ProjectChatModelCapability; children: ReactNode
}) {
    const heading = useId()
    const [label, tone] = statuses[status]
    return <Well role="group" aria-labelledby={heading} className="space-y-3 px-3 py-3">
        <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="min-w-0 space-y-0.5">
                <h3 id={heading} className="text-sm font-semibold">{name}</h3>
                <p className="text-xs text-muted-foreground [&_code]:font-mono">{detail}</p>
            </div>
            <StatusPill tone={tone}>{label}</StatusPill>
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
            <CapabilityRow name="Chat" capability={chat} />
            <CapabilityRow name="Flows" capability={flow} />
        </dl>
        {problem && <div className="space-y-1 border-l-2 border-destructive/60 pl-3 text-sm">
            <p><span className="font-medium">Problem: </span><span className="whitespace-pre-wrap">{problem}</span></p>
            {next && <p><span className="font-medium">What to do: </span>{next}</p>}
        </div>}
        {children}
    </Well>
}

type EnvRow = { key: string; value: string; hidden: boolean; saved: boolean }
type Draft = { id: string; name: string; executable: string; args: string; permission_policy: 'allow' | 'deny'; env: EnvRow[]; channel: boolean; channelArgs: string; channelEnv: EnvRow[]; channelConfig: string }

const strings = (value: unknown) => Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
const record = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
const lines = (text: string) => text.split('\n').map((line) => line.replace(/\r$/, '')).filter((line) => line.trim())
const savedRows = (value: unknown): EnvRow[] => Object.keys(record(value)).map((key) => ({ key, value: '', hidden: true, saved: true }))

function draftFor(id: string | null, entry: AcpHarnessEntry | null): Draft {
    const registration = entry?.registration ?? {}
    const channel = registration.system_instructions ? record(registration.system_instructions) : null
    return {
        id: id ?? '', name: typeof registration.name === 'string' ? registration.name : '', executable: typeof registration.executable === 'string' ? registration.executable : '',
        args: strings(registration.args).join('\n'), permission_policy: registration.permission_policy === 'deny' ? 'deny' : 'allow', env: savedRows(registration.env),
        channel: !!channel, channelArgs: strings(channel?.args).join('\n'), channelEnv: savedRows(channel?.env),
        channelConfig: Object.entries(record(channel?.config)).map(([key, value]) => `${key}=${String(value)}`).join('\n'),
    }
}

function registrationFor(draft: Draft): AcpHarnessRegistration {
    return {
        name: draft.name.trim(), executable: draft.executable.trim(), args: lines(draft.args), permission_policy: draft.permission_policy,
        env: Object.fromEntries(draft.env.map((row) => [row.key.trim(), row.hidden ? HIDDEN_ENV_VALUE : row.value])),
        ...(draft.channel ? { system_instructions: {
            args: lines(draft.channelArgs),
            env: Object.fromEntries(draft.channelEnv.map((row) => [row.key.trim(), { text: row.hidden ? HIDDEN_ENV_VALUE : row.value }])),
            config: Object.fromEntries(lines(draft.channelConfig).map((line) => [line.slice(0, line.indexOf('=')).trim(), line.slice(line.indexOf('=') + 1).trim()])),
        } } : {}),
    }
}

function draftErrors(draft: Draft, isNew: boolean, taken: string[]): Partial<Record<'id' | 'name' | 'executable' | 'env' | 'channelEnv' | 'channelConfig', string>> {
    const envError = (rows: EnvRow[]) => {
        const keys = rows.map((row) => row.key.trim())
        const bad = keys.find((key) => !key || key.includes('='))
        if (bad !== undefined) return 'Each variable needs a name without “=”.'
        const reserved = keys.find((key) => reservedEnv.includes(key))
        if (reserved) return `${reserved} is set by Spark and can’t be changed.`
        return new Set(keys).size === keys.length ? '' : 'Each variable name can appear once.'
    }
    return Object.fromEntries(Object.entries({
        id: !isNew ? '' : !/^[a-z0-9][a-z0-9_-]{0,63}$/.test(draft.id) ? 'Use 1–64 lowercase letters, digits, “-” or “_”, starting with a letter or digit.' : taken.includes(draft.id) ? 'Another harness already uses this ID.' : '',
        name: draft.name.trim() ? '' : 'Enter a name.',
        executable: draft.executable.trim() ? '' : 'Enter the program that starts the agent.',
        env: envError(draft.env),
        channelEnv: draft.channel ? envError(draft.channelEnv) : '',
        channelConfig: draft.channel && lines(draft.channelConfig).some((line) => line.indexOf('=') < 1 || !line.slice(line.indexOf('=') + 1).trim()) ? 'Write each setting as id=value.' : '',
    }).filter(([, message]) => message))
}

function AcpHarnessForm({ id, entry, view, onCancel, onSaved }: {
    id: string | null; entry: AcpHarnessEntry | null; view: AgentSettingsView | null; onCancel: () => void; onSaved: (view: AgentSettingsView, name: string) => void
}) {
    const [initial] = useState(() => draftFor(id, entry))
    const [draft, setDraft] = useState(initial)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    // A new harness shows what is missing once Add is tried; an existing one at once.
    const [tried, setTried] = useState(id !== null)
    const prefix = useId()
    const dirty = JSON.stringify(draft) !== JSON.stringify(initial)
    useSettingsNavigationProtection(dirty, pending, 'harness changes')
    const found = draftErrors(draft, id === null, Object.keys(view?.acp_harnesses ?? {}))
    const invalid = Object.keys(found).length > 0
    const errors = tried ? found : {}
    const set = (patch: Partial<Draft>) => setDraft((current) => ({ ...current, ...patch }))
    const save = async () => {
        setTried(true)
        if (!view || invalid) return
        setPending(true); setError('')
        try {
            onSaved(await saveAcpHarness(view.revision, draft.id, registrationFor(draft)), draft.name.trim())
        } catch (error) { setError(errorText(error, 'Unable to save the harness.')) } finally { setPending(false) }
    }
    const field = (key: keyof typeof errors) => ({ id: `${prefix}-${key}`, 'aria-invalid': !!errors[key], 'aria-describedby': errors[key] ? `${prefix}-${key}-error` : undefined })
    const fieldError = (key: keyof typeof errors) => errors[key] && <FieldError id={`${prefix}-${key}-error`}>{errors[key]}</FieldError>
    return <Well role="group" aria-label={id ? `Edit ${initial.name || id}` : 'Add ACP harness'} className="space-y-3 px-3 py-3">
        <h3 className="text-sm font-semibold">{id ? `Edit ${initial.name || id}` : 'Add an ACP harness'}</h3>
        <fieldset disabled={pending} className="space-y-3">
            {id === null && <Field>
                <FieldLabel htmlFor={`${prefix}-id`}>ID</FieldLabel>
                <Input {...field('id')} value={draft.id} placeholder="my-agent" autoComplete="off" onChange={(event) => set({ id: event.target.value })} />
                <p className="text-xs text-muted-foreground">Chats and flows choose it as acp:{draft.id || 'id'}. It can’t be changed later.</p>
                {fieldError('id')}
            </Field>}
            <Field>
                <FieldLabel htmlFor={`${prefix}-name`}>Name</FieldLabel>
                <Input {...field('name')} value={draft.name} onChange={(event) => set({ name: event.target.value })} />
                {fieldError('name')}
            </Field>
            <Field>
                <FieldLabel htmlFor={`${prefix}-executable`}>Program</FieldLabel>
                <Input {...field('executable')} value={draft.executable} placeholder="/usr/local/bin/my-agent" autoComplete="off" onChange={(event) => set({ executable: event.target.value })} />
                <p className="text-xs text-muted-foreground">A full path, or a name found on PATH. Spark runs it directly, not through a shell.</p>
                {fieldError('executable')}
            </Field>
            <Field>
                <FieldLabel htmlFor={`${prefix}-args`}>Arguments</FieldLabel>
                <Textarea id={`${prefix}-args`} value={draft.args} placeholder="--acp" onChange={(event) => set({ args: event.target.value })} />
                <p className="text-xs text-muted-foreground">One per line.</p>
            </Field>
            <EnvRows label="Environment" rows={draft.env} secret error={errors.env} errorId={`${prefix}-env-error`} onChange={(env) => set({ env })}
                help="Set for the agent’s process, over what it inherits. Saved values stay hidden; replace one to change it." />
            <Field>
                <FieldLabel htmlFor={`${prefix}-permissions`}>Permission requests</FieldLabel>
                <NativeSelect id={`${prefix}-permissions`} value={draft.permission_policy} onChange={(event) => set({ permission_policy: event.target.value as Draft['permission_policy'] })}>
                    <option value="allow">Allow</option><option value="deny">Deny</option>
                </NativeSelect>
                <p className="text-xs text-muted-foreground">How Spark answers when the agent asks permission to use a tool.</p>
            </Field>
            <details open={initial.channel}>
                <SettingsSummary>Pinned instructions</SettingsSummary>
                <div className="space-y-3 pt-2">
                    <p className="text-xs text-muted-foreground">For an agent that can’t take ACP system prompts: how Spark passes a chat’s or flow’s pinned instructions when it starts the agent. Write {'{{instructions}}'} for the text, or {'{{instructions_file}}'} for the path of a file holding it.</p>
                    <Label className="flex items-center gap-2 text-sm"><Switch checked={draft.channel} onCheckedChange={(channel) => set({ channel })} />Pass pinned instructions when starting the agent</Label>
                    {draft.channel && <>
                        <Field>
                            <FieldLabel htmlFor={`${prefix}-channel-args`}>Extra arguments</FieldLabel>
                            <Textarea id={`${prefix}-channel-args`} value={draft.channelArgs} placeholder="--system-prompt-file={{instructions_file}}" onChange={(event) => set({ channelArgs: event.target.value })} />
                            <p className="text-xs text-muted-foreground">One per line.</p>
                        </Field>
                        <EnvRows label="Environment templates" rows={draft.channelEnv} error={errors.channelEnv} errorId={`${prefix}-channelEnv-error`} onChange={(channelEnv) => set({ channelEnv })}
                            help="Variables whose values carry the instructions. Saved values stay hidden." />
                        <Field>
                            <FieldLabel htmlFor={`${prefix}-channelConfig`}>Required session settings</FieldLabel>
                            <Textarea {...field('channelConfig')} value={draft.channelConfig} placeholder="mode=custom" onChange={(event) => set({ channelConfig: event.target.value })} />
                            <p className="text-xs text-muted-foreground">Session settings the agent needs so it reads the instructions, one id=value per line.</p>
                            {fieldError('channelConfig')}
                        </Field>
                    </>}
                </div>
            </details>
        </fieldset>
        <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={pending || (tried && invalid) || (!dirty && id !== null)} onClick={() => void save()}>{id ? 'Save' : 'Add harness'}</Button>
            <Button size="sm" variant="ghost" disabled={pending} onClick={onCancel}>Cancel</Button>
        </div>
        {pending && <p role="status" className="text-xs text-muted-foreground">Saving…</p>}
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    </Well>
}

function EnvRows({ label, help, rows, secret = false, error, errorId, onChange }: {
    label: string; help: string; rows: EnvRow[]; secret?: boolean; error?: string; errorId: string; onChange: (rows: EnvRow[]) => void
}) {
    const update = (index: number, patch: Partial<EnvRow>) => onChange(rows.map((row, at) => at === index ? { ...row, ...patch } : row))
    return <fieldset className="space-y-2" aria-describedby={error ? errorId : undefined}>
        <legend className="text-sm font-medium">{label}</legend>
        <p className="text-xs text-muted-foreground">{help}</p>
        {rows.map((row, index) => {
            const name = row.key.trim() || `variable ${index + 1}`
            return <div key={index} className="flex flex-wrap items-center gap-2">
                <Input className="w-48" aria-label={`Name of ${label.toLowerCase()} variable ${index + 1}`} value={row.key} disabled={row.saved} placeholder="NAME" autoComplete="off"
                    onChange={(event) => update(index, { key: event.target.value })} />
                {row.hidden
                    ? <><span className="text-sm text-muted-foreground">Value hidden</span><Button size="sm" variant="outline" aria-label={`Replace value of ${name}`} onClick={() => update(index, { hidden: false, value: '' })}>Replace value</Button></>
                    : <>
                        <Input className="min-w-0 flex-1" type={secret ? 'password' : 'text'} autoComplete="off" aria-label={`Value of ${name}`} value={row.value} onChange={(event) => update(index, { value: event.target.value })} />
                        {row.saved && <Button size="sm" variant="ghost" aria-label={`Keep saved value of ${name}`} onClick={() => update(index, { hidden: true, value: '' })}>Keep saved value</Button>}
                    </>}
                <Button size="sm" variant="ghost" aria-label={`Remove ${name}`} onClick={() => onChange(rows.filter((_, at) => at !== index))}>Remove</Button>
            </div>
        })}
        <Button size="sm" variant="outline" onClick={() => onChange([...rows, { key: '', value: '', hidden: false, saved: false }])}>Add variable</Button>
        {error && <FieldError id={errorId}>{error}</FieldError>}
    </fieldset>
}
