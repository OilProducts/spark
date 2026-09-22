import { useEffect, useRef, useState } from 'react'
import { Check, Copy } from 'lucide-react'

import { Button } from '@/components/ui/button'

export function TranscriptCopyButton({ label, text }: { label: string; text: string }) {
    const [status, setStatus] = useState<'idle' | 'copied' | 'failed'>('idle')
    const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

    useEffect(() => () => {
        if (resetTimer.current !== null) clearTimeout(resetTimer.current)
    }, [])

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(text)
            if (resetTimer.current !== null) clearTimeout(resetTimer.current)
            setStatus('copied')
            resetTimer.current = setTimeout(() => {
                setStatus('idle')
                resetTimer.current = null
            }, 2_000)
        } catch {
            setStatus('failed')
        }
    }

    return (
        <span className="inline-flex items-center gap-1">
            <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                aria-label={status === 'copied' ? `${label} copied` : label}
                onClick={() => void copy()}
                className="text-muted-foreground"
            >
                {status === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
            </Button>
            {status === 'failed' ? (
                <span role="status" className="text-xs text-destructive">Copy failed.</span>
            ) : null}
        </span>
    )
}
