import { useCallback, useEffect, useRef, useState } from 'react'

import { fetchUsage, type UsageHarness, type UsageResponse } from '@/lib/api/usageApi'

export type HarnessUsage = UsageResponse | { error: string }

const REFRESH_MS = 5 * 60_000

/**
 * Each harness account's plan usage, checked on mount, every five minutes while Spark is on
 * screen, and on `check()`. Each harness fails on its own.
 */
export function useUsage(harnesses: UsageHarness[]) {
    const [usage, setUsage] = useState<Partial<Record<UsageHarness, HarnessUsage>>>({})
    const checkedAt = useRef(0)
    const checking = useRef(false)
    const check = useCallback(() => {
        if (checking.current) return
        checking.current = true
        checkedAt.current = Date.now()
        void Promise.all(harnesses.map((harness) => fetchUsage(harness).then(
            (response): [UsageHarness, HarnessUsage] => [harness, response],
            (error: unknown): [UsageHarness, HarnessUsage] => [harness, { error: error instanceof Error ? error.message : String(error) }],
        ))).then((results) => {
            checking.current = false
            setUsage(Object.fromEntries(results))
        })
    }, [harnesses])
    useEffect(() => {
        const checkIfStale = () => {
            if (!document.hidden && Date.now() - checkedAt.current >= REFRESH_MS) check()
        }
        const onSettingsEvent = (event: Event) => {
            const section = (event as CustomEvent | undefined)?.detail?.payload?.section
            if (!section || section === 'agents' || section === 'codex' || section === 'claude_code') check()
        }
        check()
        const timer = window.setInterval(checkIfStale, REFRESH_MS)
        document.addEventListener('visibilitychange', checkIfStale)
        window.addEventListener('spark:settings-live-event', onSettingsEvent)
        return () => {
            window.clearInterval(timer)
            document.removeEventListener('visibilitychange', checkIfStale)
            window.removeEventListener('spark:settings-live-event', onSettingsEvent)
        }
    }, [check])
    return { usage, check }
}
