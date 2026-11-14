import { useEffect, useState } from 'react'

/**
 * The current time for elapsed-time labels. While `live`, it re-renders every second, so a
 * running clock advances even when no data arrives; otherwise data updates drive re-renders.
 */
export function useNow(live: boolean): number {
    const [, setTick] = useState(0)
    useEffect(() => {
        if (!live) return
        const timer = window.setInterval(() => setTick((tick) => tick + 1), 1000)
        return () => window.clearInterval(timer)
    }, [live])
    // eslint-disable-next-line react-hooks/purity -- a render-time clock; the tick above only schedules re-renders
    return Date.now()
}
