import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { codexConnectionRequest, type CodexConnection } from './services/codexConnection'

export function CodexConnectionControls() {
    const [connection, setConnection] = useState<CodexConnection | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const pending = connection?.status === 'pending'

    useEffect(() => {
        const controller = new AbortController()
        void codexConnectionRequest('status', controller.signal).then((next) => {
            if (!controller.signal.aborted) setConnection(next)
        }).catch((error: unknown) => {
            if (!controller.signal.aborted) setError(error instanceof Error ? error.message : 'Unable to check Codex connection.')
        })
        return () => controller.abort()
    }, [])

    useEffect(() => {
        if (!pending || busy || error) return
        const controller = new AbortController()
        const timer = window.setTimeout(() => {
            void codexConnectionRequest('status', controller.signal).then((next) => {
                if (controller.signal.aborted) return
                setConnection(next)
                if (next.status === 'connected') window.dispatchEvent(new Event('spark:codex-connected'))
            }).catch((error: unknown) => {
                if (!controller.signal.aborted) setError(error instanceof Error ? error.message : 'Unable to check sign-in. Check the connection to try again.')
            })
        }, 1000)
        return () => { window.clearTimeout(timer); controller.abort() }
    }, [pending, busy, error, connection])

    const request = async (action: 'browser' | 'device' | 'cancel' | 'status') => {
        setBusy(true)
        setError(null)
        try {
            const next = await codexConnectionRequest(action)
            setConnection(next)
            if (next.status === 'connected') window.dispatchEvent(new Event('spark:codex-connected'))
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to connect Codex.')
        } finally { setBusy(false) }
    }

    const disabled = busy || (!connection && !error)
    return <div className="space-y-3 text-xs">
        <p>Connect your ChatGPT account for Codex in Spark. Codex refreshes this connection automatically.</p>
        <p role="status" aria-live="polite">
            {!connection ? (error ? 'Connection unavailable.' : 'Checking connection…')
                : pending ? 'Waiting for sign-in…'
                    : connection.status === 'connected'
                        ? `Connected${connection.account?.email ? ` as ${connection.account.email}` : ''}${connection.account?.plan ? ` (${connection.account.plan})` : ''}. You can retry your message or run.`
                        : 'Your Codex connection needs sign-in.'}
        </p>
        {connection?.message && <p>{connection.message}</p>}
        {pending && connection.login_url && <div className="space-y-2">
            {connection.user_code && <p>Enter this code: <strong className="select-all font-mono text-sm">{connection.user_code}</strong></p>}
            <a className="font-medium underline" href={connection.login_url} target="_blank" rel="noopener noreferrer" onClick={(event) => {
                if (isTauri()) {
                    event.preventDefault()
                    void openUrl(connection.login_url!).catch(() => setError('Unable to open the browser. Copy the sign-in link and open it in your browser.'))
                }
            }}>Continue sign-in in your browser</a>
        </div>}
        <div className="flex flex-wrap gap-2">
            {pending ? <Button size="sm" variant="outline" disabled={busy} onClick={() => void request('cancel')}>Cancel sign-in</Button>
                : <>
                    <Button size="sm" disabled={disabled} onClick={() => void request('browser')}>
                        {connection?.status === 'connected' ? 'Reconnect Codex' : 'Connect Codex'}
                    </Button>
                    <Button size="sm" variant="outline" disabled={disabled} onClick={() => void request('device')}>Use a device code</Button>
                </>}
            <Button size="sm" variant="outline" disabled={disabled} onClick={() => void request('status')}>Check connection</Button>
        </div>
        {!pending && <p className="text-muted-foreground">For Spark running in Docker or on another computer, use a device code.</p>}
        {error && <p role="alert" className="text-destructive">{error}</p>}
    </div>
}

export function CodexConnectionSettings() {
    return <Card className="gap-4 py-4 shadow-sm">
        <CardHeader className="px-4"><CardTitle className="text-sm">Codex connection</CardTitle></CardHeader>
        <CardContent className="px-4"><CodexConnectionControls /></CardContent>
    </Card>
}

export function CodexReconnect() {
    return <Dialog>
        <DialogTrigger asChild><Button size="sm" variant="outline" className="mt-2">Reconnect Codex</Button></DialogTrigger>
        <DialogContent>
            <DialogHeader>
                <DialogTitle>Reconnect Codex</DialogTitle>
                <DialogDescription>Your conversation is saved. Sign in, then close this dialog and retry your message or run.</DialogDescription>
            </DialogHeader>
            <CodexConnectionControls />
        </DialogContent>
    </Dialog>
}
