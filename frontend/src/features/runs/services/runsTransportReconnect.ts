import { useEffect, useState } from 'react'

const RUNS_TRANSPORT_RECONNECT_EVENT = 'spark:runs-transport-reconnect'
export const RUNS_TRANSPORT_LOST_EVENT = 'spark:runs-transport-lost'

export function requestRunsTransportReconnect() {
    window.dispatchEvent(new Event(RUNS_TRANSPORT_RECONNECT_EVENT))
}

/** The live stream can't reach Spark: runs say what they show may be out of date until it reconnects. */
export function reportRunsTransportLost() {
    window.dispatchEvent(new Event(RUNS_TRANSPORT_LOST_EVENT))
}

export function useRunsTransportReconnectSignal(enabled = true) {
    const [signal, setSignal] = useState(0)

    useEffect(() => {
        if (!enabled) {
            return
        }
        const handleReconnect = () => {
            setSignal((current) => current + 1)
        }
        window.addEventListener(RUNS_TRANSPORT_RECONNECT_EVENT, handleReconnect)
        return () => {
            window.removeEventListener(RUNS_TRANSPORT_RECONNECT_EVENT, handleReconnect)
        }
    }, [enabled])

    return signal
}
