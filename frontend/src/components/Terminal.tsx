import { useStore } from "@/store"
import { TerminalSquare } from "lucide-react"
import { useEffect, useRef, useState } from "react"

export function Terminal() {
    const { viewMode } = useStore()
    const [logs, setLogs] = useState<{ time: string; msg: string; type: 'info' | 'success' | 'error' }[]>([])
    const logsEndRef = useRef<HTMLDivElement>(null)

    useEffect(() => {
        // Connect to WebSocket proxy
        const wsProtocol = window.location.protocol === 'https:' ? 'wss' : 'ws'
        const ws = new WebSocket(`${wsProtocol}://${window.location.host}/ws`)

        ws.onmessage = (e) => {
            try {
                const data = JSON.parse(e.data)
                if (data.type === "log") {
                    const isSuccess = data.msg.toLowerCase().includes("success")
                    const isError = /fail|error|⚠️/i.test(data.msg)

                    setLogs(prev => [...prev, {
                        time: new Date().toLocaleTimeString('en-GB', { hour12: false }),
                        msg: data.msg,
                        type: isSuccess ? 'success' : isError ? 'error' : 'info'
                    }])
                }
            } catch {
                // ignore
            }
        }

        return () => {
            ws.close()
        }
    }, [])

    useEffect(() => {
        logsEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    }, [logs])

    if (viewMode !== 'execution') return null

    return (
        <footer className="h-72 border-t bg-background flex flex-col z-30 shadow-[0_-10px_40px_rgba(0,0,0,0.1)]">
            <div className="h-10 border-b flex items-center justify-between px-4 bg-muted/30 shrink-0">
                <div className="flex items-center gap-2">
                    <TerminalSquare className="w-4 h-4 text-muted-foreground" />
                    <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Terminal Output</span>
                </div>
                <button onClick={() => setLogs([])} className="h-7 px-2 text-xs font-medium text-muted-foreground hover:text-foreground">
                    Clear
                </button>
            </div>
            <div className="flex-1 p-4 overflow-y-auto font-mono text-sm space-y-1">
                {logs.map((log, i) => (
                    <div key={i} className="flex gap-4 py-0.5 border-b border-border/50 last:border-0 hover:bg-muted/50 rounded px-2">
                        <span className="text-muted-foreground w-20 shrink-0 select-none">{log.time}</span>
                        <span className={`break-all ${log.type === 'success' ? 'text-green-500' :
                            log.type === 'error' ? 'text-destructive' :
                                'text-foreground'
                            }`}>
                            {log.msg}
                        </span>
                    </div>
                ))}
                <div ref={logsEndRef} />
            </div>
        </footer>
    )
}
