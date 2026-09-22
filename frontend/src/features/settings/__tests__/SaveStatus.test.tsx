import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { SaveStatus } from '../SaveStatus'

afterEach(() => { cleanup(); vi.useRealTimers() })

it('shows Saved until the draft is dirty or the timer expires, keeps notices, and shows errors as alerts', () => {
    vi.useFakeTimers()
    const { rerender } = render(<SaveStatus message="Saved." error="" dirty={false} />)
    expect(screen.getByRole('status')).toHaveTextContent('Saved.')
    expect(screen.getByRole('status')).toHaveClass('text-success')
    rerender(<SaveStatus message="Saved." error="" dirty />)
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<SaveStatus message="Saved." error="" dirty={false} />)
    act(() => vi.advanceTimersByTime(4000))
    expect(screen.queryByRole('status')).toBeNull()
    rerender(<SaveStatus message="Settings changed elsewhere." error="Conflict" dirty />)
    act(() => vi.advanceTimersByTime(4000))
    expect(screen.getByRole('status')).toHaveTextContent('Settings changed elsewhere.')
    expect(screen.getByRole('alert')).toHaveTextContent('Conflict')
})
