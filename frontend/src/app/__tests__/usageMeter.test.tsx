import { ActivityBar, untilText } from '@/app/ActivityBar'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'

afterEach(() => vi.unstubAllGlobals())

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

it('shows each harness account\'s plan limits, and a harness that cannot report on its own', async () => {
    const resetsAt = new Date(Date.now() + (2 * 24 + 3) * 3_600_000 + 30_000).toISOString()
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input)
        if (url.endsWith('/claude/usage')) return json({ windows: [
            { label: 'Weekly', used_percent: 33, resets_at: resetsAt },
            { label: 'Weekly · Fable', used_percent: 92, resets_at: null },
        ] })
        if (url.endsWith('/codex/usage')) return json({ detail: 'Your Codex connection needs sign-in.' }, 503)
        return json({})
    }))
    const user = userEvent.setup()
    render(<ActivityBar />)

    const meter = screen.getByTestId('usage-meter')
    // The bar names each harness's tightest window.
    await vi.waitFor(() => expect(meter).toHaveAccessibleName('Usage: Claude Code 92% used, Codex unavailable'))
    await user.click(meter)
    const [claude, codex] = within(screen.getByTestId('usage-meter-list')).getAllByTestId('usage-meter-harness')
    expect(within(claude).getAllByTestId('usage-meter-window').map((row) => row.textContent)).toEqual([
        'Weekly33% used · resets in 2d 3h',
        'Weekly · Fable92% used',
    ])
    expect(within(codex).getByRole('alert')).toHaveTextContent('Your Codex connection needs sign-in.')
})

it('counts down to a reset in days, hours or minutes', () => {
    const now = Date.parse('2026-11-20T00:00:00Z')
    expect(untilText('2026-11-23T04:10:00Z', now)).toBe('3d 4h')
    expect(untilText('2026-11-20T02:05:00Z', now)).toBe('2h 5m')
    expect(untilText('2026-11-20T00:12:00Z', now)).toBe('12m')
    expect(untilText('2026-11-19T00:00:00Z', now)).toBe('0m')
})
