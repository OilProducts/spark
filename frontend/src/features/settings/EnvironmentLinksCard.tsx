import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import { flushSync } from 'react-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { useDialogController } from '@/components/app/dialog-controller'
import { InlineError } from '@/components/app/inline-error'
import { Well } from '@/components/app/well'
import {
    getActiveEnvironmentId,
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

/** `revoked` marks a revoke the server confirmed before a listing supplied its time. */
type TokenRow = PeerToken & { revoked?: boolean }

const formField = (data: FormData, name: string) => String(data.get(name) ?? '').trim()

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)

/** Relinking an environment ID rewrites its row, but can keep its origin and, within a second, its created_at. */
const sameLink = (a: EnvironmentLink, b: EnvironmentLink) =>
    a.environment_id === b.environment_id && a.origin === b.origin && a.label === b.label && a.created_at === b.created_at

const firstEnabledIn = (scope: Element | null) => scope?.querySelector<HTMLElement>('button:enabled, input:enabled') ?? null

/** A mutation the server confirmed: a revoked token ID, a created link, or a removed link's environment ID. */
type Mutation = { revoked: string } | { linked: EnvironmentLink } | { unlinked: string }

// Leaving Settings unmounts the card and switching environments remounts it, so a request can settle after its card
// is gone. It then reaches the cards mounted now: those for its environment show a mutation and refresh, and a failure
// shows in the current card, or in the next one to mount.
type LiveCard = { environmentId: string | null; settle: (mutation: Mutation) => void; showError: (message: string) => void }
const liveCards = new Set<LiveCard>()
let undeliveredError: string | null = null

const msLeft = (receipt: PairingCodeReceipt) => Date.parse(receipt.expires_at) - Date.now()

/** Drops the code once the receipt expires; the expiry stays for the notice. An unreadable expiry counts as expired. */
const expireReceipt = (receipt: PairingCodeReceipt | null) =>
    receipt?.code && !(msLeft(receipt) > 0) ? { ...receipt, code: '' } : receipt

export function EnvironmentLinksCard({ active = true }: { active?: boolean }) {
    const { activeEnvironmentId } = useActiveEnvironment()
    const isLocalEnvironment = activeEnvironmentId === null
    const [descriptor, setDescriptor] = useState<EnvironmentDescriptor | null>(null)
    const [tokens, setTokens] = useState<TokenRow[]>([])
    const [links, setLinks] = useState<EnvironmentLink[]>([])
    const [tokensLoaded, setTokensLoaded] = useState(false)
    const [linksLoaded, setLinksLoaded] = useState(false)
    const [loading, setLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [status, setStatus] = useState('')
    const [pairingCode, setPairingCode] = useState<PairingCodeReceipt | null>(null)
    const [busy, setBusy] = useState(false)
    const [projectsByEnv, setProjectsByEnv] = useState<Record<string, ProjectLookup>>({})
    const refreshSeqRef = useRef(0)
    // Counts link listings a removal's check showed, so a refresh started before one drops only its links result.
    const linksSeqRef = useRef(0)
    // Counts errors shown by actions, so a refresh leaves visible any error reported after it started.
    const errorCountRef = useRef(0)
    // The Projects lookup, cached or in flight, per environment; removing or replacing its link drops it so its answer is ignored.
    const lookupsRef = useRef(new Map<string, EnvironmentLink>())
    const liveCardRef = useRef<LiveCard | null>(null)
    const { confirm } = useDialogController()

    const showError = useCallback((message: string) => {
        errorCountRef.current += 1
        setStatus('')
        setError(message)
    }, [])

    const forgetProjects = useCallback((environmentId: string) => {
        lookupsRef.current.delete(environmentId)
        setProjectsByEnv((prev) => {
            const next = { ...prev }
            delete next[environmentId]
            return next
        })
    }, [])

    // Shows a link listing and drops Projects results for links it no longer has as they were when looked up.
    const showListedLinks = useCallback((listed: EnvironmentLink[]) => {
        for (const [environmentId, ticket] of lookupsRef.current) {
            if (!listed.some((row) => sameLink(row, ticket))) forgetProjects(environmentId)
        }
        setLinks(listed)
    }, [forgetProjects])

    const refresh = useCallback(async () => {
        const seq = ++refreshSeqRef.current
        const linksSeq = linksSeqRef.current
        const errorCount = errorCountRef.current
        setPairingCode(expireReceipt)
        setLoading(true)
        const [descriptorResult, tokensResult, linksResult] = await Promise.allSettled([
            fetchEnvironmentDescriptor(),
            fetchPeerTokens(),
            fetchEnvironmentLinks(),
        ])
        if (seq !== refreshSeqRef.current) return
        const errors: string[] = []
        if (descriptorResult.status === 'fulfilled') setDescriptor(descriptorResult.value)
        else errors.push(`Could not load this environment: ${errorText(descriptorResult.reason, 'unknown error.')}`)
        if (tokensResult.status === 'fulfilled') setTokens(tokensResult.value)
        else errors.push(`Could not load peer tokens: ${errorText(tokensResult.reason, 'unknown error.')}`)
        setTokensLoaded(tokensResult.status === 'fulfilled')
        if (linksSeq === linksSeqRef.current) {
            if (linksResult.status === 'fulfilled') showListedLinks(linksResult.value)
            else errors.push(`Could not load linked environments: ${errorText(linksResult.reason, 'unknown error.')}`)
            setLinksLoaded(linksResult.status === 'fulfilled')
        }
        // An error an action reported after this refresh started stays, beside any listing errors.
        const newerError = errorCountRef.current !== errorCount
        if (errors.length > 0) {
            setStatus('')
            setError((prev) => [newerError ? prev : null, ...errors].filter(Boolean).join(' '))
        } else if (!newerError) {
            setError(null)
        }
        setLoading(false)
    }, [showListedLinks])

    // Shows a confirmed mutation at once, even if the follow-up listing fails, and retires refreshes started before it.
    const applyMutation = useCallback((mutation: Mutation) => {
        refreshSeqRef.current += 1
        if ('revoked' in mutation) {
            setTokens((prev) => prev.map((row) => row.token_id === mutation.revoked ? { ...row, revoked: true } : row))
            return
        }
        const environmentId = 'linked' in mutation ? mutation.linked.environment_id : mutation.unlinked
        forgetProjects(environmentId)
        setLinks((prev) => [
            ...prev.filter((row) => row.environment_id !== environmentId),
            ...('linked' in mutation ? [mutation.linked] : []),
        ])
        // Only a listing can say no links remain.
        if ('unlinked' in mutation) setLinksLoaded(false)
    }, [forgetProjects])

    useEffect(() => {
        if (!active) return
        void refresh()
    }, [active, refresh])

    useEffect(() => {
        // A hidden card refreshes once shown instead.
        const card = {
            environmentId: activeEnvironmentId,
            settle: (mutation: Mutation) => {
                applyMutation(mutation)
                if (active) void refresh()
            },
            showError,
        }
        liveCards.add(card)
        liveCardRef.current = card
        if (undeliveredError) showError(undeliveredError)
        undeliveredError = null
        return () => {
            liveCards.delete(card)
            liveCardRef.current = null
        }
    }, [active, activeEnvironmentId, applyMutation, refresh, showError])

    // Expire the receipt on time while mounted; refresh() re-checks it whenever the card becomes active.
    // A timer can fire before expires_at if the wall clock moves back, so it re-arms until the receipt has expired.
    useEffect(() => {
        if (!pairingCode?.code) return
        let timer = 0
        const check = () => {
            const left = msLeft(pairingCode)
            // setTimeout fires at once for delays past 2^31-1 ms.
            if (left > 0) timer = window.setTimeout(check, Math.min(left, 2 ** 31 - 1))
            else setPairingCode(expireReceipt)
        }
        check()
        return () => window.clearTimeout(timer)
    }, [pairingCode])

    const copyText = async (label: string, value: string) => {
        if (!navigator.clipboard?.writeText) {
            showError(`Clipboard is unavailable; could not copy ${label}.`)
            return
        }
        try {
            await navigator.clipboard.writeText(value)
            setStatus(`Copied ${label}.`)
            setError(null)
        } catch {
            showError(`Could not copy ${label}.`)
        }
    }

    const copyPairingCode = (receipt: PairingCodeReceipt) => {
        const current = expireReceipt(receipt)
        if (current?.code) void copyText('pairing code', current.code)
        else setPairingCode(current)
    }

    // Shows an action's failure here, or, once this card is gone, in the card showing now or the next one to mount.
    const reportError = (message: string) => {
        const shown = liveCardRef.current ?? liveCards.values().next().value
        if (shown) shown.showError(message)
        else undeliveredError = [undeliveredError, message].filter(Boolean).join(' ')
    }

    // Confirms, then runs an action with the card busy. The shared dialog has no trigger of its own to refocus, so focus
    // returns to the control that opened it once the dialog closes or the action settles, or, if that control is gone, to
    // the first enabled control in its section, else in the card. Focus the user has moved elsewhere meanwhile stays put.
    const confirmThenRun = async (
        trigger: HTMLElement,
        options: Parameters<typeof confirm>[0],
        failure: string,
        action: () => Promise<void>,
    ) => {
        const section = trigger.closest('section')
        const card = trigger.closest('[data-slot="card"]')
        if (!(await confirm(options))) {
            window.setTimeout(() => trigger.focus(), 0)
            return
        }
        setBusy(true)
        setError(null)
        try {
            await action()
        } catch (err) {
            reportError(errorText(err, failure))
        } finally {
            flushSync(() => setBusy(false))
            if (!document.activeElement || document.activeElement === document.body) {
                const target = trigger.isConnected ? trigger : firstEnabledIn(section) ?? firstEnabledIn(card)
                target?.focus()
            }
        }
    }

    const settleMutation = async (message: string, mutation: Mutation) => {
        applyMutation(mutation)
        setStatus(message)
        for (const card of liveCards) {
            if (card !== liveCardRef.current && card.environmentId === activeEnvironmentId) card.settle(mutation)
        }
        // A card that is gone would list whichever environment is active now.
        if (liveCardRef.current) await refresh()
    }

    const handleCreateCode = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        const form = event.currentTarget
        const label = formField(new FormData(form), 'code_label')
        setBusy(true)
        setError(null)
        try {
            const receipt = await createPairingCode(label)
            setPairingCode(receipt)
            setStatus('Created a single-use pairing code.')
            form.reset()
        } catch (err) {
            reportError(errorText(err, 'Could not create pairing code.'))
        } finally {
            setBusy(false)
        }
    }

    const handleRevoke = (token: PeerToken, trigger: HTMLElement) => confirmThenRun(trigger, {
        title: 'Revoke peer token?',
        description: `${token.label ? `${token.label} (${token.token_id})` : token.token_id} will stop authenticating requests to this environment.`,
        confirmLabel: 'Revoke token',
        cancelLabel: 'Cancel',
        confirmVariant: 'destructive',
    }, 'Could not revoke token.', async () => {
        await revokePeerToken(token.token_id)
        await settleMutation(`Revoked token ${token.label || token.token_id}.`, { revoked: token.token_id })
    })

    const handleAddLink = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        const form = event.currentTarget
        const data = new FormData(form)
        const origin = formField(data, 'link_origin')
        const expected_id = formField(data, 'link_expected_id')
        const code = formField(data, 'link_code')
        const label = formField(data, 'link_label')
        if (!origin || !expected_id || !code) {
            showError('Enter the remote origin, expected environment ID, and pairing code.')
            return
        }
        setBusy(true)
        setError(null)
        try {
            const created = await addEnvironmentLink({ origin, expected_id, code, label })
            form.reset()
            if (isLocalEnvironment) notifyEnvironmentLinksChanged()
            await settleMutation(`Linked ${created.label || created.environment_id}.`, { linked: created })
        } catch (err) {
            reportError(errorText(err, 'Could not add environment link.'))
        } finally {
            setBusy(false)
        }
    }

    const handleRemoveLink = (link: EnvironmentLink, trigger: HTMLElement) => {
        const name = link.label || link.environment_id
        return confirmThenRun(trigger, {
            title: 'Remove environment link?',
            description: `${name} (${link.origin}) will be unlinked. Its open delegations pause until you link it again.`,
            confirmLabel: 'Remove link',
            cancelLabel: 'Cancel',
            confirmVariant: 'destructive',
        }, 'Could not remove link.', async () => {
            // DELETE goes by environment ID alone, and relinking that ID (here or in another tab) replaces its row, so only
            // delete the link the user confirmed if the server still has it as it was.
            // ponytail: a relink landing between this check and the DELETE, or one matching the row's origin, label and
            // second, still loses; closing that needs a conditional DELETE on the server, which this change leaves out of scope.
            const refreshesBefore = refreshSeqRef.current
            const latest = await fetchEnvironmentLinks()
            // Requests go to whichever environment is active when sent, and the user can switch while the check runs.
            if (getActiveEnvironmentId() !== activeEnvironmentId) {
                const from = descriptor?.display_name || 'the previous environment'
                throw new Error(`You switched environments before the link to ${name} on ${from} was removed, so nothing was removed.`)
            }
            const current = latest.find((row) => row.environment_id === link.environment_id)
            if (!current || !sameLink(current, link)) {
                // A refresh started during the check has a newer listing. Otherwise show this one in place of the links an
                // older refresh still in flight returns; that refresh still shows its other results and ends loading.
                if (refreshSeqRef.current === refreshesBefore) {
                    linksSeqRef.current += 1
                    showListedLinks(latest)
                    setLinksLoaded(true)
                }
                throw new Error(`The link to ${name} changed since you chose to remove it, so nothing was removed. Review the current link before removing it again.`)
            }
            await removeEnvironmentLink(link.environment_id)
            if (isLocalEnvironment) notifyEnvironmentLinksChanged()
            await settleMutation(`Removed link to ${name}.`, { unlinked: link.environment_id })
        })
    }

    const handleInspectProjects = async (link: EnvironmentLink) => {
        const existing = projectsByEnv[link.environment_id]
        if (existing?.projects) {
            forgetProjects(link.environment_id)
            return
        }
        const lookup = { ...link }
        lookupsRef.current.set(link.environment_id, lookup)
        setProjectsByEnv((prev) => ({ ...prev, [link.environment_id]: { loading: true } }))
        try {
            const projects = await fetchLinkProjects(link.environment_id)
            if (lookupsRef.current.get(link.environment_id) !== lookup) return
            setProjectsByEnv((prev) => ({ ...prev, [link.environment_id]: { projects } }))
        } catch (err) {
            if (lookupsRef.current.get(link.environment_id) !== lookup) return
            setStatus('')
            setProjectsByEnv((prev) => ({
                ...prev,
                [link.environment_id]: { error: errorText(err, 'Could not list projects.') },
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
                    {pairingCode?.code ? (
                        <Well className="space-y-1.5 px-3 py-2">
                            <div className="flex flex-wrap items-center justify-between gap-2">
                                <span className="text-xs font-medium text-foreground">Single-use pairing code (shown once)</span>
                                <Button size="sm" variant="outline" onClick={() => copyPairingCode(pairingCode)}>Copy code</Button>
                            </div>
                            <div className="break-all font-mono text-xs text-foreground" data-testid="pairing-code-value">{pairingCode.code}</div>
                            <div className="text-xs text-muted-foreground">Expires at {pairingCode.expires_at}</div>
                        </Well>
                    ) : pairingCode ? (
                        <p className="text-xs text-muted-foreground">The pairing code expired at {pairingCode.expires_at}. Create a new one to pair.</p>
                    ) : null}
                </div>

                <section className="space-y-2">
                    <h3 className="text-sm font-medium text-foreground">Issued peer tokens</h3>
                    {tokens.length === 0 ? (
                        tokensLoaded ? <p className="text-xs text-muted-foreground">No peer tokens issued yet.</p> : null
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
                                            {token.revoked_at || token.revoked ? (
                                                <span className="text-xs text-muted-foreground">Revoked</span>
                                            ) : token.is_current ? (
                                                <div className="flex items-center gap-2">
                                                    <span className="text-xs text-muted-foreground">Current connection</span>
                                                    <Button
                                                        size="sm"
                                                        variant="ghost"
                                                        aria-label={`Cannot revoke ${name}: it authenticates this connection`}
                                                        title="This token authenticates this connection; revoke it from another environment or this machine."
                                                        disabled
                                                    >
                                                        Revoke
                                                    </Button>
                                                </div>
                                            ) : (
                                                <Button size="sm" variant="ghost" aria-label={`Revoke ${name}`} disabled={busy} onClick={(event) => void handleRevoke(token, event.currentTarget)}>
                                                    Revoke
                                                </Button>
                                            )}
                                        </Well>
                                    </li>
                                )
                            })}
                        </ul>
                    )}
                </section>

                <section className="space-y-3">
                    <h3 className="text-sm font-medium text-foreground">Linked environments</h3>
                    {links.length === 0 ? (
                        linksLoaded ? <p className="text-xs text-muted-foreground">No linked environments configured.</p> : null
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
                                                    <Button size="sm" variant="ghost" aria-label={`Remove link ${name}`} disabled={busy} onClick={(event) => void handleRemoveLink(link, event.currentTarget)}>
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
                </section>

                {status ? <p role="status" className="text-xs text-muted-foreground">{status}</p> : null}
                {error ? <InlineError>{error}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
