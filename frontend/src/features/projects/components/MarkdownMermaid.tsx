import { useEffect, useRef, useState, type ReactNode } from 'react'

let mermaidPromise: Promise<typeof import('mermaid')['default']> | undefined
let nextDiagramId = 0
const loadMermaid = () => mermaidPromise ??= import('mermaid').then(({ default: mermaid }) => {
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', suppressErrorRendering: true })
    return mermaid
})

export function MarkdownMermaid({ source, children }: { source: string; children: ReactNode }) {
    const [result, setResult] = useState<{ source: string; svg: string | null }>({ source, svg: null })
    if (result.source !== source) setResult({ source, svg: null })
    const running = useRef(false)
    const pending = useRef<{ source: string; cancelled: boolean } | null>(null)

    useEffect(() => {
        const request = { source, cancelled: false }
        pending.current = request
        const drain = async () => {
            if (running.current) return
            running.current = true
            try {
                while (pending.current) {
                    const current = pending.current
                    pending.current = null
                    try {
                        const mermaid = await loadMermaid()
                        if (current.cancelled) continue
                        const { svg } = await mermaid.render(`spark-mermaid-${++nextDiagramId}`, current.source)
                        if (!current.cancelled) setResult({ source: current.source, svg })
                    } catch {
                        if (!current.cancelled) setResult({ source: current.source, svg: null })
                    }
                }
            } finally {
                running.current = false
            }
        }
        void drain()
        return () => { request.cancelled = true }
    }, [source])

    return result.source === source && result.svg ? (
        <>
            <div className="markdown-diagram" role="img" aria-label="Mermaid diagram" dangerouslySetInnerHTML={{ __html: result.svg }} />
            <details><summary>Diagram source</summary>{children}</details>
        </>
    ) : children
}
