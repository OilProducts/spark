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
import { TranscriptCopyButton } from '@/components/app/transcript/TranscriptCopyButton'
import { useStore } from '@/store'
import { CodexConnectionControls } from './CodexConnectionSettings'
import { ClaudeCodeConnectionControls } from './ClaudeCodeConnectionSettings'
import { SettingsSummary } from './SettingsSummary'
import { SaveStatus } from './SaveStatus'
import { useSettingsNavigationProtection } from './hooks/useSettingsNavigationProtection'
import { cancelAcpSignIn, fetchAgentSettings, fetchHarnessCatalog, HIDDEN_ENV_VALUE, saveAcpHarness, saveAgentSettings, signInAcpHarness, type AcpHarnessEntry, type AcpHarnessRegistration, type AgentSettingsView,
    type HarnessSetupStatus, type ProjectChatModelCapability, type ProjectChatModelProviderStatusResponse, type ProjectChatModelsResponse } from './services/executionSettings'

type Connection = 'connected' | 'disconnected' | 'pending' | 'error'
type Status = HarnessSetupStatus | 'signed_out' | 'signing_in' | 'checking' | 'unchecked'
type Provider = ProjectChatModelProviderStatusResponse | undefined

const statuses: Record<Status, [string, StatusTone]> = {
    ready: ['Ready', 'success'],
    checking: ['Checking…', 'muted'],
    unchecked: ['Not checked', 'muted'],
    signing_in: ['Signing in…', 'info'],
    signed_out: ['Signed out', 'warning'],
    unauthenticated: ['Signed out', 'warning'],
    not_installed: ['Not installed', 'danger'],
    unreachable: ['Not responding', 'danger'],
    misconfigured: ['Misconfigured', 'danger'],
}

const capabilityNames: Record<string, string> = {
    model_selection: 'Model choice', reasoning_effort: 'Reasoning effort', session_resume: 'Resuming sessions',
    plan_mode: 'Plan mode', steering: 'Messages during a turn',
    child_intervention: 'Subagent control', forms: 'Questions and forms', per_prompt_usage: 'Token usage',
}
const stateWords = { supported: 'Yes', unsupported: 'No', unverified: 'Unverified' } as const
// Statuses the user has something to fix.
const attention: Status[] = ['misconfigured', 'not_installed', 'unreachable', 'unauthenticated', 'signed_out']
// A word a shell reads as it is, else quoted.
const shellWord = (word: string) => /^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replaceAll("'", `'\\''`)}'`
// The words as one shell command; leading `NAME=value` words stay assignments, with only the value quoted.
const shellCommand = (words: string[]) => {
    let assigning = true
    return words.map((word) => {
        const assignment = assigning ? /^([A-Za-z_]\w*)=(.*)$/s.exec(word) : null
        assigning = !!assignment
        return assignment ? `${assignment[1]}=${shellWord(assignment[2])}` : shellWord(word)
    }).join(' ')
}
// Spark sets these for every agent process.
const reservedEnv = ['SPARK_HOME', 'SPARK_API_BASE_URL', 'SPARK_PROJECT_WORK_DIR']

const errorText = (error: unknown, fallback: string) => error instanceof Error ? error.message : fallback
const record = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
const displayName = (id: string, registration: Partial<AcpHarnessRegistration>) => typeof registration.name === 'string' && registration.name.trim() ? registration.name : id

const natives = {
    codex: { name: 'Codex', key: 'codex_binary', program: 'codex', install: 'Install Codex' },
    'claude-code': { name: 'Claude Code', key: 'claude_binary', program: 'claude', install: 'Install Claude Code' },
} as const

function nativeStatus(provider: Provider, connection: Connection | undefined, failed: boolean): Status {
    if (!provider) return failed ? 'unchecked' : 'checking'
    if (provider.status === 'checking') return 'checking'
    if (provider.setup_status === 'not_installed') return 'not_installed'
    if (connection === 'pending') return 'signing_in'
    if (connection === 'disconnected') return 'signed_out'
    if (provider.status === 'unavailable' || connection === 'error') return 'unreachable'
    return connection === 'connected' ? 'ready' : 'checking'
}

function acpStatus(entry: AcpHarnessEntry, provider: Provider, failed: boolean): Status {
    if (entry.error) return 'misconfigured'
    return provider?.setup_status ?? (failed ? 'unchecked' : 'checking')
}

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
    // The ACP harnesses being edited, by ID; '' is a new one.
    const [editing, setEditing] = useState<string[]>([])
    const [message, setMessage] = useState('')
    const [error, setError] = useState('')
    const { confirm } = useDialogController()
    const revision = useRef<string | null>(null)
    const request = useRef(0)

    const show = useCallback((next: AgentSettingsView) => { revision.current = next.revision; setView(next); setViewError('') }, [])
    const load = useCallback(() => fetchAgentSettings().then(show, (error: unknown) => setViewError(errorText(error, 'Unable to load harness settings.'))), [show])
    // Each harness shows its last check until the server has a newer one.
    // `quiet` shows what the server found since, without checking anything.
    const check = useCallback((refresh: boolean, quiet = false) => {
        const current = ++request.current
        if (!quiet) {
            setChecking(true)
            // Sign-in is read again with the programs the check finds.
            setChecks((value) => value + 1)
        }
        return fetchHarnessCatalog(null, refresh).then((next) => {
            if (current === request.current) { setCatalog(next); setCatalogError('') }
            return next
        }, (error: unknown) => {
            // A failed check leaves every harness unchecked, not as the last check found it.
            if (current === request.current) { setCatalog(null); setCatalogError(errorText(error, 'Unable to check harnesses.')) }
            return null
        }).finally(() => { if (current === request.current) setChecking(false) })
    }, [])
    const recheck = useCallback(() => void check(true), [check])
    const refresh = useCallback(() => void check(false), [check])
    // `provider` is the saved ACP harness, whose check result the message then gives.
    const saved = useCallback((next: AgentSettingsView, text: string, provider?: string) => {
        show(next); setError('')
        const checkingText = `${text} Checking it now…`
        setMessage(checkingText)
        void check(false).then((catalog) => {
            const found = provider && catalog?.providers[provider]?.setup_status
            setMessage((current) => current !== checkingText ? current : found ? `${text} Status: ${statuses[found][0]}.` : text)
        })
    }, [show, check])
    const onConnection = useCallback((id: string, state: Connection) => setConnections((current) => current[id] === state ? current : { ...current, [id]: state }), [])
    const edit = (id: string) => { setEditing((current) => current.includes(id) ? current : [...current, id]); setMessage('') }
    const close = (id: string) => setEditing((current) => current.filter((other) => other !== id))

    useEffect(() => {
        void load()
        void check(false)
        const changed = (event: Event) => {
            const payload = (event as CustomEvent<{ payload?: { section?: string; revision?: unknown } }>).detail?.payload
            if (!payload?.section || !['agents', 'codex', 'claude_code'].includes(payload.section)) return
            // This card's own saves already show their result.
            if (payload.section === 'agents' && payload.revision === revision.current) return
            void load()
            void check(false)
        }
        // A harness check ended on the server.
        const checked = () => void check(false, true)
        window.addEventListener('spark:settings-live-event', changed)
        window.addEventListener('spark:harness-checked', checked)
        return () => {
            window.removeEventListener('spark:settings-live-event', changed)
            window.removeEventListener('spark:harness-checked', checked)
        }
    }, [load, check])

    const remove = async (id: string, name: string) => {
        if (!view || !await confirm({ title: `Remove ${name}?`, description: `Chats and flows that use acp:${id} will stop working until it is added again.`, confirmLabel: 'Remove', cancelLabel: 'Cancel', confirmVariant: 'destructive' })) return
        setError('')
        try { show(await saveAcpHarness(view.revision, id, null)); setMessage(`Removed ${name}.`); void check(false) } catch (error) { setError(errorText(error, `Unable to remove ${name}.`)) }
    }

    const providers: Partial<ProjectChatModelsResponse['providers']> = catalog?.providers ?? {}
    const failed = !!catalogError
    const nativeRows = (['codex', 'claude-code'] as const).map((id) => [id, nativeStatus(providers[id], connections[id], failed)] as const)
    const acpRows = Object.entries(view?.acp_harnesses ?? {}).map(([id, entry]) => [id, entry, acpStatus(entry, providers[`acp:${id}`], failed)] as const)
    // In the order statuses are listed, ready first.
    const counts = new Map(Object.values(statuses).map(([label]) => [label.toLowerCase(), 0]))
    for (const status of [...nativeRows.map(([, status]) => status), ...acpRows.map(([, , status]) => status)]) {
        const label = statuses[status][0].toLowerCase()
        counts.set(label, (counts.get(label) ?? 0) + 1)
    }
    const needing = [...nativeRows.map(([id, status]) => [id, natives[id].name, status] as const),
        ...acpRows.map(([id, entry, status]) => [`acp:${id}`, displayName(id, entry.registration), status] as const)].filter(([, , status]) => attention.includes(status))
    const jump = (id: string) => {
        const row = document.getElementById(`harness-${id}`)
        row?.scrollIntoView({ block: 'start' }); row?.focus()
    }
    return <Card className="gap-4 py-4">
        <CardHeader className="gap-1 px-4"><h2 className="text-base font-semibold">Agent harnesses</h2></CardHeader>
        <CardContent className="space-y-3 px-4 pt-0">
            <p className="text-sm text-muted-foreground">The agents that run your chats and flows. Codex and Claude Code are built in; add any agent that speaks the Agent Client Protocol (ACP).</p>
            <p className="text-sm font-medium" aria-label="Harness overview">{[...counts].filter(([, count]) => count).map(([label, count]) => `${count} ${label}`).join(' · ')}</p>
            {needing.length > 0 && <nav aria-label="Harnesses to fix" className="flex flex-wrap items-center gap-x-1 gap-y-0.5 text-sm">
                <span className="text-muted-foreground">To fix:</span>
                {needing.map(([id, name, status]) => <Button key={id} size="sm" variant="link" className="h-auto px-1 py-0" onClick={() => jump(id)}>{name} ({statuses[status][0].toLowerCase()})</Button>)}
            </nav>}
            {viewError && <p role="alert" className="text-sm text-destructive">{viewError}</p>}
            {view && !view.stored && <div role="alert" className="space-y-1 text-sm text-destructive">
                <p>Agent settings can’t be read, so programs can’t be changed here and nothing can be checked: {view.validation_errors?.join(' ')}</p>
                <Button size="sm" variant="outline" onClick={() => {
                    useStore.getState().setSettingsCategory('execution')
                    requestAnimationFrame(() => document.getElementById('agent-session-limits')?.scrollIntoView({ block: 'start' }))
                }}>Fix it in Execution › Agent session limits</Button>
            </div>}
            {catalogError && <p role="alert" className="text-sm text-destructive">Unable to check harnesses: {catalogError}</p>}
            {nativeRows.map(([id, status]) => <NativeHarness key={id} id={id} view={view} provider={providers[id]} status={status}
                onConnection={onConnection} checking={checking} checks={checks} onCheck={recheck} onSignedIn={refresh} onSaved={saved} />)}
            {acpRows.map(([id, entry, status]) => editing.includes(id)
                ? <AcpHarnessForm key={id} id={id} entry={entry} view={view} onCancel={() => close(id)} onSaved={(next, name) => { close(id); saved(next, `Saved ${name}.`, `acp:${id}`) }} />
                : <AcpHarness key={id} id={id} entry={entry} provider={providers[`acp:${id}`]} status={status} checking={checking} onCheck={recheck} onSignedIn={refresh}
                    onEdit={() => edit(id)} onRemove={(name) => void remove(id, name)} />)}
            {editing.includes('') ? <AcpHarnessForm id={null} entry={null} view={view} onCancel={() => close('')} onSaved={(next, name, added) => { close(''); saved(next, `Added ${name}.`, `acp:${added}`) }} />
                : <Button variant="outline" disabled={!view} onClick={() => edit('')}>Add ACP harness</Button>}
            <SaveStatus message={message} error={error} />
        </CardContent>
    </Card>
}

function NativeHarness({ id, view, provider, status, onConnection, checking, checks, onCheck, onSignedIn, onSaved }: {
    id: keyof typeof natives; view: AgentSettingsView | null; provider: Provider; status: Status; onConnection: (id: string, state: Connection) => void
    checking: boolean; checks: number; onCheck: () => void; onSignedIn: () => void; onSaved: (view: AgentSettingsView, message: string) => void
}) {
    const { name, key, program, install } = natives[id]
    const stored = view?.stored?.native?.[key] ?? ''
    const [draft, setDraft] = useState<string | null>(null)
    const value = draft ?? stored
    const dirty = draft !== null && draft.trim() !== stored
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    useSettingsNavigationProtection(dirty, pending, `${name} program`)
    const last = useRef<Connection | null>(null)
    const onStatus = useCallback((state: Connection) => {
        // A finished sign-in checks the harness again.
        if (state === 'connected' && last.current === 'pending') onSignedIn()
        last.current = state
        onConnection(id, state)
    }, [id, onConnection, onSignedIn])
    const save = async () => {
        if (!view?.stored) return
        setPending(true); setError('')
        try {
            const next = await saveAgentSettings(view.revision, { ...view.stored, native: { ...view.stored.native, [key]: value.trim() || null } })
            setDraft(null)
            onSaved(next, `Saved the ${name} program.`)
        } catch (error) { setError(errorText(error, `Unable to save the ${name} program.`)) } finally { setPending(false) }
    }
    const next = status === 'not_installed' ? `${install}, or set Program below to the full path of its ${program} executable, then check again.`
        : status === 'signed_out' ? `Sign in below. Spark checks it again when you finish.`
            : status === 'unreachable' ? `Check that Program below is ${name}'s ${program} executable and that it starts from a terminal, then check again.` : ''
    const configured = view?.effective?.native?.[key]
    return <HarnessRow anchor={id} name={name} kind="Built in" command={configured ?? program} status={status}
        problem={status === 'signed_out' ? `${name} is not signed in.` : status === 'not_installed' ? `Its program, ${configured ?? `${program} on PATH`}, wasn’t found.`
            : status === 'unreachable' ? provider?.error ?? null : null} next={next}
        more="Program and sign-in"
        actions={<Button size="sm" variant="outline" aria-label={`Check ${name} again`} disabled={checking} onClick={onCheck}>Check again</Button>}>
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
        {/* A missing program has nothing to sign in to; read sign-in once the program is checked. */}
        {provider?.setup_status && provider.setup_status !== 'not_installed' && (id === 'codex'
            ? <CodexConnectionControls key={checks} onStatus={onStatus} showCheck={false} />
            : <ClaudeCodeConnectionControls key={checks} onStatus={onStatus} showCheck={false} />)}
    </HarnessRow>
}

function acpNextStep(status: Status, signIn: 'spark' | 'terminal' | null): string {
    switch (status) {
        case 'misconfigured': return 'Edit the registration to fix it, or remove it.'
        case 'not_installed': return 'Install the agent, or edit Program to the full path of its executable, then check again.'
        case 'unauthenticated': return signIn === 'spark' ? 'Sign in below with a method the agent offers. Spark checks it again when you finish.'
            : signIn === 'terminal' ? 'Run the sign-in command below in a terminal and finish what it asks, then check again.'
                : 'The agent names no way to sign in. Sign in with its own login command, or edit its environment to give it the credentials it reads (such as an API key), then check again.'
        case 'unreachable': return 'Check its program, arguments and environment against the agent’s instructions for ACP, then check again.'
        default: return ''
    }
}

function AcpHarness({ id, entry, provider, status: checked, checking, onCheck, onSignedIn, onEdit, onRemove }: {
    id: string; entry: AcpHarnessEntry; provider: Provider; status: Status; checking: boolean
    onCheck: () => void; onSignedIn: () => void; onEdit: () => void; onRemove: (name: string) => void
}) {
    const registration = entry.registration
    const name = displayName(id, registration)
    const [signing, setSigning] = useState(false)
    const [signInError, setSignInError] = useState('')
    const cancelled = useRef(false)
    const status: Status = signing ? 'signing_in' : checked
    const args = typeof registration.args === 'string' ? [registration.args] : Array.isArray(registration.args) ? registration.args : []
    const command = [registration.executable, ...args].filter((part) => typeof part === 'string').join(' ')
    const env = Object.keys(record(registration.env))
    const capabilities = Object.entries(provider?.capabilities ?? {})
    const offered = status === 'unauthenticated' || signing ? provider?.auth_methods ?? [] : []
    const methods = offered.filter((method) => !method.terminal)
    const terminal = offered.filter((method) => method.terminal)
    const signIn = async (method: string) => {
        cancelled.current = false
        setSigning(true); setSignInError('')
        try {
            const result = await signInAcpHarness(id, method)
            if (result.setup_status !== 'ready' && !cancelled.current) setSignInError(`Sign-in didn’t finish: ${result.error ?? 'the agent is still signed out.'}`)
        } catch (error) { if (!cancelled.current) setSignInError(errorText(error, `Unable to sign in to ${name}.`)) } finally { setSigning(false); onSignedIn() }
    }
    const cancel = () => {
        cancelled.current = true
        void cancelAcpSignIn(id).catch((error: unknown) => setSignInError(errorText(error, 'Unable to cancel the sign-in.')))
    }
    const fix = (offered.length > 0 || signInError) && <div className="space-y-2 pt-1">
        {methods.map((method) => <div key={method.id} className="space-y-0.5">
            <Button size="sm" disabled={signing || checking} aria-label={`Sign in to ${name} with ${method.name}`} onClick={() => void signIn(method.id)}>Sign in with {method.name}</Button>
            {method.description && <p className="text-xs text-muted-foreground">{method.description}</p>}
        </div>)}
        {terminal.map((method) => {
            const command = shellCommand(method.terminal!)
            return <div key={method.id} className="space-y-0.5">
                <p className="font-medium">{method.name}, in a terminal:</p>
                <div className="flex items-center gap-1"><code className="min-w-0 break-all rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{command}</code><TranscriptCopyButton label={`Copy ${method.name} command`} text={command} /></div>
                {method.description && <p className="text-xs text-muted-foreground">{method.description}</p>}
                {env.length > 0 && <p className="text-xs text-muted-foreground">Spark starts this agent with {env.join(', ')} set. Their values are hidden here, so if its sign-in needs them, set them in the terminal yourself first.</p>}
            </div>
        })}
        {signing && <div className="space-y-1">
            <p role="status" className="text-xs">Finish signing in where {name} asks, such as a browser window it opens. Spark waits up to 3 minutes.</p>
            <Button size="sm" variant="outline" onClick={cancel}>Cancel sign-in</Button>
        </div>}
        {signInError && !signing && <p role="alert" className="text-destructive">{signInError}</p>}
    </div>
    const more = env.length > 0 || (status === 'ready' && capabilities.length > 0)
    return <HarnessRow anchor={`acp:${id}`} name={name} kind={`ACP · acp:${id}`} command={command} status={status}
        problem={entry.error ?? (['checking', 'ready', 'signing_in'].includes(status) ? null : provider?.error || null)}
        next={acpNextStep(status, methods.length > 0 ? 'spark' : terminal.length > 0 ? 'terminal' : null)} fix={fix || null}
        more={more ? 'Details' : undefined}
        actions={<>
            <Button size="sm" variant="outline" aria-label={`Edit ${name}`} onClick={onEdit}>Edit</Button>
            <Button size="sm" variant="outline" aria-label={`Check ${name} again`} disabled={checking || signing} onClick={onCheck}>Check again</Button>
            <Button size="sm" variant="ghost" aria-label={`Remove ${name}`} onClick={() => onRemove(name)}>Remove</Button>
        </>}>
        {env.length > 0 && <p className="text-xs text-muted-foreground">Environment: {env.join(', ')} (values hidden)</p>}
        {status === 'ready' && capabilities.length > 0 && <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
            {capabilities.map(([key, capability]) => <CapabilityRow key={key} name={capabilityNames[key] ?? key} capability={capability} />)}
        </dl>}
    </HarnessRow>
}

function CapabilityRow({ name, capability }: { name: string; capability: ProjectChatModelCapability }) {
    return <>
        <dt className="text-muted-foreground">{name}</dt>
        <dd>{stateWords[capability.state]}{capability.state === 'supported' ? '' : <span className="text-muted-foreground"> — {capability.reason}</span>}</dd>
    </>
}

function HarnessRow({ anchor, name, kind, command, status, problem, next, fix, actions, more, children }: {
    anchor: string; name: string; kind: string; command: string; status: Status; problem: string | null; next: string; fix?: ReactNode
    actions: ReactNode; more?: string; children?: ReactNode
}) {
    const heading = useId()
    const [label, tone] = statuses[status]
    // Details open while the harness needs fixing and fold once it is ready;
    // a check that finds what the last one did leaves them as they are.
    const [open, setOpen] = useState(false)
    const settled = status !== 'checking' && status !== 'unchecked'
    const last = useRef<Status | null>(null)
    useEffect(() => {
        if (!settled || status === last.current) return
        last.current = status
        setOpen(status !== 'ready')
    }, [settled, status])
    return <Well id={`harness-${anchor}`} tabIndex={-1} role="group" aria-labelledby={heading} className="space-y-2 px-3 py-2.5 scroll-mt-4 focus-visible:outline-2">
        <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-0.5">
                <h3 id={heading} className="text-sm font-semibold">{name}</h3>
                <p className="truncate text-xs text-muted-foreground" title={command}>{kind}{command && <> · runs <code className="font-mono">{command}</code></>}</p>
            </div>
            <StatusPill tone={tone}>{label}</StatusPill>
        </div>
        {(problem || fix) && <div className="space-y-1 border-l-2 border-destructive/60 pl-3 text-sm">
            {problem && <p><span className="font-medium">Problem: </span><span className="whitespace-pre-wrap break-words">{problem}</span></p>}
            {problem && next && <p><span className="font-medium">What to do: </span>{next}</p>}
            {fix}
        </div>}
        <div className="flex flex-wrap gap-2">{actions}</div>
        {more && <details open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
            <SettingsSummary>{more}</SettingsSummary>
            <div className="space-y-3 pt-2">{children}</div>
        </details>}
    </Well>
}

type EnvRow = { key: string; value: string; hidden: boolean; saved: boolean }
type Draft = { id: string; name: string; executable: string; args: string; permission_policy: 'allow' | 'deny'; env: EnvRow[]; channel: boolean; channelArgs: string; channelEnv: EnvRow[]; channelConfig: string }

// A hand-written string of arguments is split where a shell would, so the form shows each one.
const strings = (value: unknown) => typeof value === 'string' ? value.split(/\s+/).filter(Boolean)
    : Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
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

// Saved as stored unless edited, so arguments a line can't show, such as blank ones or ones of several lines, are kept.
const unlessEdited = <T,>(text: string, initial: string, stored: unknown, valid: (value: unknown) => value is T, read: (text: string) => T): T =>
    text === initial && valid(stored) ? stored : read(text)
const isArgs = (value: unknown): value is string[] => Array.isArray(value) && value.every((entry) => typeof entry === 'string')
const isConfig = (value: unknown): value is Record<string, string> => !!value && typeof value === 'object' && !Array.isArray(value) && Object.values(value).every((entry) => typeof entry === 'string')

function registrationFor(draft: Draft, initial: Draft, stored: Partial<AcpHarnessRegistration>): AcpHarnessRegistration {
    const channel = record(stored.system_instructions)
    return {
        name: draft.name.trim(), executable: draft.executable.trim(), args: unlessEdited(draft.args, initial.args, stored.args, isArgs, lines), permission_policy: draft.permission_policy,
        env: Object.fromEntries(draft.env.map((row) => [row.key.trim(), row.hidden ? HIDDEN_ENV_VALUE : row.value])),
        ...(draft.channel ? { system_instructions: {
            args: unlessEdited(draft.channelArgs, initial.channelArgs, channel.args, isArgs, lines),
            env: Object.fromEntries(draft.channelEnv.map((row) => [row.key.trim(), { text: row.hidden ? HIDDEN_ENV_VALUE : row.value }])),
            config: unlessEdited(draft.channelConfig, initial.channelConfig, channel.config, isConfig, (text) => Object.fromEntries(lines(text).map((line) => [line.slice(0, line.indexOf('=')).trim(), line.slice(line.indexOf('=') + 1).trim()]))),
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
    id: string | null; entry: AcpHarnessEntry | null; view: AgentSettingsView | null; onCancel: () => void; onSaved: (view: AgentSettingsView, name: string, id: string) => void
}) {
    // What was saved when editing began, to notice a save made elsewhere since.
    const [base, setBase] = useState(() => ({ saved: JSON.stringify(entry), draft: draftFor(id, entry) }))
    const initial = base.draft
    const [draft, setDraft] = useState(initial)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    // A new harness shows what is missing once Add is tried; an existing one at once.
    const [tried, setTried] = useState(id !== null)
    const prefix = useId()
    const stale = id !== null && JSON.stringify(entry) !== base.saved
    const restart = () => { const next = draftFor(id, entry); setBase({ saved: JSON.stringify(entry), draft: next }); setDraft(next); setError('') }
    // Saving a misconfigured registration as the form reads it is a fix.
    const dirty = JSON.stringify(draft) !== JSON.stringify(initial) || !!entry?.error
    useSettingsNavigationProtection(dirty, pending, 'harness changes')
    const found = draftErrors(draft, id === null, Object.keys(view?.acp_harnesses ?? {}))
    const invalid = Object.keys(found).length > 0
    const errors = tried ? found : {}
    const set = (patch: Partial<Draft>) => setDraft((current) => ({ ...current, ...patch }))
    const save = async () => {
        setTried(true)
        if (!view || invalid || stale) return
        setPending(true); setError('')
        try {
            onSaved(await saveAcpHarness(view.revision, draft.id, registrationFor(draft, initial, entry?.registration ?? {})), draft.name.trim(), draft.id)
        } catch (error) { setError(errorText(error, 'Unable to save the harness.')) } finally { setPending(false) }
    }
    const field = (key: keyof typeof errors) => ({ id: `${prefix}-${key}`, 'aria-invalid': !!errors[key], 'aria-describedby': errors[key] ? `${prefix}-${key}-error` : undefined })
    const fieldError = (key: keyof typeof errors) => errors[key] && <FieldError id={`${prefix}-${key}-error`}>{errors[key]}</FieldError>
    return <Well role="group" aria-label={id ? `Edit ${initial.name || id}` : 'Add ACP harness'} className="space-y-3 px-3 py-3">
        <h3 className="text-sm font-semibold">{id ? `Edit ${initial.name || id}` : 'Add an ACP harness'}</h3>
        {entry?.error && !stale && <p className="text-sm"><span className="font-medium">Problem: </span>{entry.error}</p>}
        {stale && <div role="alert" className="space-y-2 text-sm">
            <p className="text-destructive">This harness was changed in another window since you began editing, so saving would undo that change.</p>
            <Button size="sm" variant="outline" onClick={restart}>Start over from what’s saved</Button>
        </div>}
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
                    <p className="text-xs text-muted-foreground">Optional. Without it, an agent that can’t take ACP system prompts gets a chat’s or flow’s pinned instructions at the start of its first message. Set it to pass them when Spark starts the agent instead. Write {'{{instructions}}'} for the text, or {'{{instructions_file}}'} for the path of a file holding it.</p>
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
            <Button size="sm" disabled={pending || stale || (tried && invalid) || (!dirty && id !== null)} onClick={() => void save()}>{id ? 'Save' : 'Add harness'}</Button>
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
