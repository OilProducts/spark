import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { claudeCodeConnectionRequest, submitClaudeCodeSignInCode, type ClaudeCodeConnection } from './services/claudeCodeConnection'

/** `autoStart` begins sign-in as soon as the connection reads signed out. */
export function ClaudeCodeConnectionControls({ autoStart = false }: { autoStart?: boolean }) {
    const [connection, setConnection] = useState<ClaudeCodeConnection | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [code, setCode] = useState('')
    const pending = connection?.status === 'pending'

    const request = async (action: 'login' | 'cancel' | 'status' | 'code') => {
        setBusy(true)
        setError(null)
        try {
            setConnection(action === 'code' ? await submitClaudeCodeSignInCode(code) : await claudeCodeConnectionRequest(action))
            if (action === 'code') setCode('')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to sign in to Claude.')
        } finally { setBusy(false) }
    }

    useEffect(() => {
        const controller = new AbortController()
        void claudeCodeConnectionRequest('status', controller.signal).then((next) => {
            if (controller.signal.aborted) return
            setConnection(next)
            if (autoStart && next.status === 'disconnected') void request('login')
        }).catch((error: unknown) => {
            if (!controller.signal.aborted) setError(error instanceof Error ? error.message : 'Unable to check Claude Code sign-in.')
        })
        return () => controller.abort()
        // eslint-disable-next-line react-hooks/exhaustive-deps -- read once on open
    }, [])

    useEffect(() => {
        if (!pending || busy || error) return
        const controller = new AbortController()
        const timer = window.setTimeout(() => {
            void claudeCodeConnectionRequest('status', controller.signal).then((next) => {
                if (!controller.signal.aborted) setConnection(next)
            }).catch((error: unknown) => {
                if (!controller.signal.aborted) setError(error instanceof Error ? error.message : 'Unable to check sign-in. Check the connection to try again.')
            })
        }, 1000)
        return () => { window.clearTimeout(timer); controller.abort() }
    }, [pending, busy, error, connection])

    const disabled = busy || (!connection && !error)
    return <div className="space-y-3 text-sm">
        <p>Sign in to your Claude account for Claude Code in Spark. Claude Code refreshes this sign-in automatically.</p>
        <p role="status" aria-live="polite">
            {!connection ? (error ? 'Connection unavailable.' : 'Checking connection…')
                : pending ? 'Waiting for sign-in… Finish in the browser window that opened.'
                    : connection.status === 'connected'
                        ? `Signed in${connection.account?.email ? ` as ${connection.account.email}` : ''}${connection.account?.plan ? ` (${connection.account.plan})` : ''}. You can retry your message or run.`
                        : 'Claude Code needs sign-in.'}
        </p>
        {connection?.message && <p>{connection.message}</p>}
        {pending && connection.login_url && <div className="space-y-2">
            <p>No browser window? <a className="font-medium underline" href={connection.login_url} target="_blank" rel="noopener noreferrer" onClick={(event) => {
                if (isTauri()) {
                    event.preventDefault()
                    void openUrl(connection.login_url!).catch(() => setError('Unable to open the browser. Copy the sign-in link and open it in your browser.'))
                }
            }}>Open the sign-in page</a>, then paste the code it shows.</p>
            <form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); void request('code') }}>
                <Input aria-label="Sign-in code" placeholder="Paste code" autoComplete="off" value={code} onChange={(event) => setCode(event.target.value)} />
                <Button type="submit" size="sm" disabled={busy || !code.trim()}>Submit code</Button>
            </form>
        </div>}
        <div className="flex flex-wrap gap-2">
            {pending ? <Button size="sm" variant="outline" disabled={busy} onClick={() => void request('cancel')}>Cancel sign-in</Button>
                : <Button size="sm" disabled={disabled} onClick={() => void request('login')}>
                    {connection?.status === 'connected' ? 'Sign in again' : 'Sign in to Claude'}
                </Button>}
            <Button size="sm" variant="outline" disabled={disabled} onClick={() => void request('status')}>Check connection</Button>
        </div>
        {error && <p role="alert" className="text-destructive">{error}</p>}
    </div>
}

export function ClaudeCodeConnectionSettings() {
    return <Card className="gap-4 py-4">
        <CardHeader className="px-4"><h3 className="text-lg font-light">Claude Code connection</h3></CardHeader>
        <CardContent className="px-4"><ClaudeCodeConnectionControls /></CardContent>
    </Card>
}

export function ClaudeCodeReconnect() {
    return <Dialog>
        <DialogTrigger asChild><Button size="sm" variant="outline" className="mt-2">Sign in to Claude</Button></DialogTrigger>
        <DialogContent>
            <DialogHeader>
                <DialogTitle>Sign in to Claude</DialogTitle>
                <DialogDescription>Your conversation is saved. Sign in, then close this dialog and retry your message or run.</DialogDescription>
            </DialogHeader>
            <ClaudeCodeConnectionControls autoStart />
        </DialogContent>
    </Dialog>
}
