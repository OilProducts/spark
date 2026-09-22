import { useEffect, useState } from 'react'
import { InlineError } from '@/components/app/inline-error'

// One save status line for every settings card: error alert, transient "Saved…" confirmation, or a persistent notice.
export function SaveStatus({ message, error, dirty }: { message?: string, error?: string | null, dirty?: boolean }) {
    const saved = !!message?.startsWith('Saved')
    const [shown, setShown] = useState(message)
    const [expired, setExpired] = useState(false)
    if (shown !== message) { setShown(message); setExpired(false) }
    useEffect(() => {
        if (!saved) return
        const timer = window.setTimeout(() => setExpired(true), 4000)
        return () => window.clearTimeout(timer)
    }, [saved, message])
    return <>
        {error ? <InlineError>{error}</InlineError> : null}
        {message && !(saved && (dirty || expired)) ? <p role="status" className={saved ? 'text-xs text-success' : 'text-xs text-muted-foreground'}>{message}</p> : null}
    </>
}
