import { RunStream } from '@/features/runs/RunStream'
import { useStore } from '@/store'
import { act, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const resetRunStreamState = () => {
  useStore.setState((state) => ({
    ...state,
    selectedRunId: null,
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    logs: [],
    humanGate: null,
    nodeStatuses: {},
    runtimeStatus: 'idle',
  }))
}

describe('RunStream save indicator', () => {
  beforeEach(() => {
    resetRunStreamState()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.runOnlyPendingTimers()
    vi.useRealTimers()
  })

  it('shows a single saved toast and dismisses it after the fade window', () => {
    render(<RunStream />)

    act(() => {
      useStore.getState().markSaveSuccess()
    })

    expect(screen.getByTestId('global-save-state-indicator')).toHaveTextContent('Saved')

    act(() => {
      vi.advanceTimersByTime(1000)
    })

    expect(screen.getByTestId('global-save-state-indicator').className).toContain('opacity-0')

    act(() => {
      vi.advanceTimersByTime(1000)
    })

    expect(screen.queryByTestId('global-save-state-indicator')).not.toBeInTheDocument()
    expect(useStore.getState().saveState).toBe('idle')
  })
})
