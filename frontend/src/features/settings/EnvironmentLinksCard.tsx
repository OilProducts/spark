import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InlineError } from '@/components/app/inline-error'
import { Well } from '@/components/app/well'
import {
    notifyEnvironmentLinksChanged,
    selectEnvironment,
    useActiveEnvironment,
} from '@/lib/activeEnvironment'
import { useStore } from '@/store'
import {
    addEnvironmentLink,
    createPairingCode,
    fetchEnvironmentDescriptor,
    fetchEnvironmentLinks,
    fetchLinkProjects,
    fetchPeerTokens,
    removeEnvironmentLink,
    revokePeerToken,
    type EnvironmentDescriptor,
    type EnvironmentLink,
    type PairingCodeReceipt,
    type PeerToken,
} from '@/lib/api/environmentsApi'

type ProjectLookup = { loading?: boolean; projects?: string[]; error?: string }

export function EnvironmentLinksCard({ active = true }: { active?: boolean }) {
    const { activeEnvironmentId } = useActiveEnvironment()
    const isLocalEnvironment = activeEnvironmentId === null
    const [descriptor, setDescriptor] = useState<EnvironmentDescriptor | null>(null)
    const [tokens, setTokens] = useState<PeerToken[]>([])
    const [links, setLinks] = useState<EnvironmentLink[]>([])
    const [loading, setLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [status, setStatus] = useState('')
    const [pairingCode, setPairingCode] = useState<PairingCodeReceipt | null>(null)
    const [busy, setBusy] = useState(false)
    const [projectsByEnv, setProjectsByEnv] = useState<Record<string, ProjectLookup>>({})

    const refresh = useCallback(async () => {
        setLoading(true)
        try {
            const [nextDescriptor, nextTokens, nextLinks] = await Promise.all([
                fetchEnvironmentDescriptor(),
                fetchPeerTokens(),
                fetchEnvironmentLinks(),
            ])
            setDescriptor(nextDescriptor)
            setTokens(nextTokens)
            setLinks(nextLinks)
            setError(null)
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not load environment links.')
        } finally {
            setLoading(false)
        }
    }, [])

    useEffect(() => {
        if (!active) return
        void refresh()
    }, [active, refresh])

    const copyText = async (label: string, value: string) => {
        try {
            await navigator.clipboard?.writeText(value)
            setStatus(`Copied ${label}.`)
        } catch {
            setStatus(`Could not copy ${label}.`)
        }
    }

    const handleCreateCode = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        const form = event.currentTarget
        const label = String(new FormData(form).get('code_label') ?? '').trim()
        setBusy(true)
        setError(null)
        try {
            const receipt = await createPairingCode(label)
            setPairingCode(receipt)
            setStatus('Created a single-use pairing code.')
            form.reset()
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not create pairing code.')
        } finally {
            setBusy(false)
        }
    }

    const handleRevoke = async (token: PeerToken) => {
        setBusy(true)
        setError(null)
        try {
            await revokePeerToken(token.token_id)
            setStatus(`Revoked token ${token.label || token.token_id}.`)
            await refresh()
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not revoke token.')
        } finally {
            setBusy(false)
        }
    }

    const handleAddLink = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        const form = event.currentTarget
        const data = new FormData(form)
        const origin = String(data.get('link_origin') ?? '').trim()
        const expected_id = String(data.get('link_expected_id') ?? '').trim()
        const code = String(data.get('link_code') ?? '').trim()
        const label = String(data.get('link_label') ?? '').trim()
        if (!origin || !expected_id || !code) {
            setError('Enter the remote origin, expected environment ID, and pairing code.')
            return
        }
        setBusy(true)
        setError(null)
        try {
            const created = await addEnvironmentLink({ origin, expected_id, code, label })
            form.reset()
            setStatus(`Linked ${created.label || created.environment_id}.`)
            if (isLocalEnvironment) notifyEnvironmentLinksChanged()
            await refresh()
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not add environment link.')
        } finally {
            setBusy(false)
        }
    }

    const handleRemoveLink = async (link: EnvironmentLink) => {
        setBusy(true)
        setError(null)
        try {
            await removeEnvironmentLink(link.environment_id)
            setProjectsByEnv((prev) => {
                const next = { ...prev }
                delete next[link.environment_id]
                return next
            })
            setStatus(`Removed link to ${link.label || link.environment_id}.`)
            if (isLocalEnvironment) notifyEnvironmentLinksChanged()
            await refresh()
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not remove link.')
        } finally {
            setBusy(false)
        }
    }

    const handleInspectProjects = async (link: EnvironmentLink) => {
        const existing = projectsByEnv[link.environment_id]
        if (existing?.projects) {
            setProjectsByEnv((prev) => {
                const next = { ...prev }
                delete next[link.environment_id]
                return next
            })
            return
        }
        setProjectsByEnv((prev) => ({ ...prev, [link.environment_id]: { loading: true } }))
        try {
            const projects = await fetchLinkProjects(link.environment_id)
            setProjectsByEnv((prev) => ({ ...prev, [link.environment_id]: { projects } }))
        } catch (err) {
            setProjectsByEnv((prev) => ({
                ...prev,
                [link.environment_id]: { error: err instanceof Error ? err.message : 'Could not list projects.' },
            }))
        }
    }

    return (
        <Card className="gap-4 py-4">
            <CardHeader className="px-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                    <h2 className="text-base font-semibold">Environment pairing &amp; links</h2>
                    <Button size="sm" variant="ghost" disabled={loading || busy} onClick={() => void refresh()}>
                        Refresh
                    </Button>
                </div>
            </CardHeader>
            <CardContent className="space-y-5 px-4 text-sm">
                <p className="text-muted-foreground">
                    This environment&apos;s identity, pairing codes and tokens for incoming requests, and server-side links used for delegating tasks to other Spark machines.
                </p>

                {descriptor ? (
                    <Well className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
                        <div className="min-w-0 space-y-0.5">
                            <div className="font-medium text-foreground">
                                {descriptor.display_name} <span className="text-xs font-normal text-muted-foreground">· protocol v{descriptor.protocol_version}{descriptor.ui_api_version > 0 ? ` · UI API v${descriptor.ui_api_version}` : ''}</span>
                            </div>
                            <div className="break-all font-mono text-xs text-muted-foreground">{descriptor.environment_id}</div>
                        </div>
                        <Button size="sm" variant="outline" onClick={() => void copyText('environment ID', descriptor.environment_id)}>
                            Copy ID
                        </Button>
                    </Well>
                ) : loading ? (
                    <p role="status" className="text-muted-foreground">Loading environment…</p>
                ) : null}

                <div className="space-y-2">
                    <h3 className="text-sm font-medium text-foreground">Pairing code</h3>
                    <form className="flex flex-wrap items-end gap-2" aria-label="Create pairing code" onSubmit={(event) => void handleCreateCode(event)}>
                        <Field className="min-w-[12rem] flex-1">
                            <FieldLabel htmlFor="pair-code-label">Token label (optional)</FieldLabel>
                            <Input id="pair-code-label" name="code_label" placeholder="Build server" autoComplete="off" />
                        </Field>
                        <Button type="submit" size="sm" variant="outline" disabled={busy}>Create pairing code</Button>
                    </form>
                    {pairingCode ? (
                        <Well className="space-y-1.5 px-3 py-2">
                            <div className="flex flex-wrap items-center justify-between gap-2">
                                <span className="text-xs font-medium text-foreground">Single-use pairing code (shown once)</span>
                                <Button size="sm" variant="outline" onClick={() => void copyText('pairing code', pairingCode.code)}>Copy code</Button>
                            </div>
                            <div className="break-all font-mono text-xs text-foreground" data-testid="pairing-code-value">{pairingCode.code}</div>
                            <div className="text-xs text-muted-foreground">Expires at {pairingCode.expires_at}</div>
                        </Well>
                    ) : null}
                </div>

                <div className="space-y-2">
                    <h3 className="text-sm font-medium text-foreground">Issued peer tokens</h3>
                    {tokens.length === 0 ? (
                        <p className="text-xs text-muted-foreground">No peer tokens issued yet.</p>
                    ) : (
                        <ul className="space-y-2">
                            {tokens.map((token) => {
                                const name = token.label || token.token_id
                                return (
                                    <li key={token.token_id}>
                                        <Well className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
                                            <div className="min-w-0 space-y-0.5">
                                                <div className="font-medium text-foreground">{name}</div>
                                                <div className="break-all text-xs text-muted-foreground">
                                                    {token.token_id} · created {token.created_at}
                                                    {token.revoked_at ? ` · revoked ${token.revoked_at}` : ''}
                                                </div>
                                            </div>
                                            {token.revoked_at ? (
                                                <span className="text-xs text-muted-foreground">Revoked</span>
                                            ) : (
                                                <Button size="sm" variant="ghost" aria-label={`Revoke ${name}`} disabled={busy} onClick={() => void handleRevoke(token)}>
                                                    Revoke
                                                </Button>
                                            )}
                                        </Well>
                                    </li>
                                )
                            })}
                        </ul>
                    )}
                </div>

                <div className="space-y-3">
                    <h3 className="text-sm font-medium text-foreground">Linked environments</h3>
                    {links.length === 0 ? (
                        <p className="text-xs text-muted-foreground">No linked environments configured.</p>
                    ) : (
                        <ul className="space-y-2">
                            {links.map((link) => {
                                const name = link.label || link.environment_id
                                const lookup = projectsByEnv[link.environment_id]
                                return (
                                    <li key={link.environment_id}>
                                        <Well className="space-y-2 px-3 py-2">
                                            <div className="flex flex-wrap items-center justify-between gap-2">
                                                <div className="min-w-0 space-y-0.5">
                                                    <div className="font-medium text-foreground">{name}</div>
                                                    <div className="break-all text-xs text-muted-foreground">{link.origin} · {link.environment_id}</div>
                                                </div>
                                                <div className="flex flex-wrap gap-2">
                                                    {isLocalEnvironment ? (
                                                        <Button
                                                            size="sm"
                                                            variant="outline"
                                                            aria-label={`Switch to ${name}`}
                                                            disabled={busy}
                                                            onClick={() => void selectEnvironment(link.environment_id, {
                                                                onActivated: () => useStore.getState().setViewMode('overview'),
                                                            })}
                                                        >
                                                            Switch
                                                        </Button>
                                                    ) : null}
                                                    <Button size="sm" variant="outline" aria-label={`Projects on ${name}`} disabled={lookup?.loading || busy} onClick={() => void handleInspectProjects(link)}>
                                                        {lookup?.projects ? 'Hide projects' : lookup?.loading ? 'Loading…' : 'Projects'}
                                                    </Button>
                                                    <Button size="sm" variant="ghost" aria-label={`Remove link ${name}`} disabled={busy} onClick={() => void handleRemoveLink(link)}>
                                                        Remove
                                                    </Button>
                                                </div>
                                            </div>
                                            {lookup?.error ? <InlineError>{lookup.error}</InlineError> : null}
                                            {lookup?.projects ? (
                                                lookup.projects.length === 0 ? (
                                                    <p className="text-xs text-muted-foreground">No registered projects on this environment.</p>
                                                ) : (
                                                    <ul className="space-y-1 border-t border-border pt-2 font-mono text-xs text-muted-foreground">
                                                        {lookup.projects.map((path) => <li key={path} className="break-all">{path}</li>)}
                                                    </ul>
                                                )
                                            ) : null}
                                        </Well>
                                    </li>
                                )
                            })}
                        </ul>
                    )}

                    <form className="space-y-3" aria-label="Link another environment" onSubmit={(event) => void handleAddLink(event)}>
                        <Field>
                            <FieldLabel htmlFor="link-origin">Remote origin</FieldLabel>
                            <Input id="link-origin" name="link_origin" placeholder="https://build.example:8000" autoComplete="off" />
                        </Field>
                        <Field>
                            <FieldLabel htmlFor="link-expected-id">Remote environment ID</FieldLabel>
                            <Input id="link-expected-id" name="link_expected_id" autoComplete="off" />
                        </Field>
                        <Field>
                            <FieldLabel htmlFor="link-code">Pairing code</FieldLabel>
                            <Input id="link-code" name="link_code" autoComplete="off" />
                        </Field>
                        <Field>
                            <FieldLabel htmlFor="link-label">Link label (optional)</FieldLabel>
                            <Input id="link-label" name="link_label" autoComplete="off" />
                        </Field>
                        <Button type="submit" size="sm" disabled={busy}>Add link</Button>
                    </form>
                </div>

                {status ? <p role="status" className="text-xs text-muted-foreground">{status}</p> : null}
                {error ? <InlineError>{error}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
