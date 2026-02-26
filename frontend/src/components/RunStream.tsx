import { useEffect } from 'react'
import { useStore } from '@/store'

function classifyLog(message: string): 'info' | 'success' | 'error' {
    const lower = message.toLowerCase()
    const isSuccess = lower.includes('success')
    const isError = /fail|error|⚠️/i.test(message)
    if (isSuccess) return 'success'
    if (isError) return 'error'
    return 'info'
}

export function RunStream() {
    const addLog = useStore((state) => state.addLog)
    const clearLogs = useStore((state) => state.clearLogs)
    const setNodeStatus = useStore((state) => state.setNodeStatus)
    const setHumanGate = useStore((state) => state.setHumanGate)
    const clearHumanGate = useStore((state) => state.clearHumanGate)
    const resetNodeStatuses = useStore((state) => state.resetNodeStatuses)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)

    useEffect(() => {
        resetNodeStatuses()
        clearHumanGate()
        clearLogs()
        if (!selectedRunId) {
            setRuntimeStatus('idle')
        }
    }, [selectedRunId, resetNodeStatuses, clearHumanGate, clearLogs, setRuntimeStatus])

    useEffect(() => {
        fetch('/status')
            .then((res) => res.json())
            .then((data) => {
                const runId = typeof data?.last_run_id === 'string' ? data.last_run_id : null
                if (!selectedRunId && runId) {
                    setSelectedRunId(runId)
                }
                if (data?.status && (!selectedRunId || runId === selectedRunId)) {
                    setRuntimeStatus(data.status)
                }
            })
            .catch(() => null)
    }, [selectedRunId, setRuntimeStatus, setSelectedRunId])

    useEffect(() => {
        if (!selectedRunId) return

        fetch(`/pipelines/${encodeURIComponent(selectedRunId)}`)
            .then((res) => (res.ok ? res.json() : null))
            .then((data) => {
                if (data?.status) {
                    setRuntimeStatus(data.status)
                }
            })
            .catch(() => null)

        const source = new EventSource(`/pipelines/${encodeURIComponent(selectedRunId)}/events`)

        source.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data)
                if (data.type === 'log') {
                    addLog({
                        time: new Date().toLocaleTimeString('en-GB', { hour12: false }),
                        msg: data.msg,
                        type: classifyLog(data.msg),
                    })
                }
                if (data.type === 'state' && data.node && data.status) {
                    setNodeStatus(data.node, data.status)
                    const currentGate = useStore.getState().humanGate
                    if (data.status !== 'waiting' && currentGate?.nodeId === data.node) {
                        clearHumanGate()
                    }
                }
                if (data.type === 'human_gate') {
                    setNodeStatus(data.node_id, 'waiting')
                    setHumanGate({
                        id: data.question_id,
                        runId: selectedRunId,
                        nodeId: data.node_id,
                        prompt: data.prompt,
                        options: data.options || [],
                        flowName: data.flow_name,
                    })
                }
                if (data.type === 'run_meta') {
                    resetNodeStatuses()
                    clearHumanGate()
                    setRuntimeStatus('running')
                }
                if (data.type === 'runtime' && data.status) {
                    setRuntimeStatus(data.status)
                }
            } catch {
                // ignore malformed events
            }
        }

        return () => {
            source.close()
        }
    }, [selectedRunId, addLog, setNodeStatus, clearHumanGate, resetNodeStatuses, setHumanGate, setRuntimeStatus])

    return null
}
