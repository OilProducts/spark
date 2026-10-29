import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { saveFlowContent } from '@/lib/flowPersistence'
import { useStore } from '@/store'

vi.mock('@/lib/flowPersistence', () => ({
  saveFlowContent: vi.fn(),
}))

function SchedulerHarness() {
  const [count, setCount] = useState(0)
  const { scheduleSave } = useFlowSaveScheduler<{ value: number }>({
    flowName: 'demo.yaml',
    debounceMs: 25,
    buildContent: (payload, flowName) => ({ flow: { id: flowName, value: payload?.value ?? null } }),
  })

  return (
    <button
      type="button"
      onClick={() => {
        setCount((current) => {
          const next = current + 1
          scheduleSave({ value: next })
          return next
        })
      }}
    >
      Count {count}
    </button>
  )
}

describe('useFlowSaveScheduler', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.mocked(saveFlowContent).mockReset()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('supports scheduling a save from inside a state updater without crashing', async () => {
    useStore.setState({ activeProjectPath: '/tmp/project-a', editorMode: 'structured' })
    render(<SchedulerHarness />)

    fireEvent.click(screen.getByRole('button', { name: 'Count 0' }))
    // The save stays with the project it was scheduled in.
    useStore.setState({ activeProjectPath: '/tmp/project-b' })

    expect(screen.getByRole('button', { name: 'Count 1' })).toBeVisible()
    expect(saveFlowContent).not.toHaveBeenCalled()

    vi.advanceTimersByTime(25)

    expect(saveFlowContent).toHaveBeenCalledTimes(1)
    expect(saveFlowContent).toHaveBeenCalledWith(
      'demo.yaml',
      { flow: { id: 'demo.yaml', value: 1 } },
      '/tmp/project-a',
    )
  })

  it('drops a structured save that lands after the switch to Files mode, by timer or unmount', () => {
    useStore.setState({ activeProjectPath: '/tmp/project-a', editorMode: 'structured' })
    const first = render(<SchedulerHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Count 0' }))
    useStore.setState({ editorMode: 'files' })
    vi.advanceTimersByTime(25)
    first.unmount()

    render(<SchedulerHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Count 0' }))
    cleanup()

    expect(saveFlowContent).not.toHaveBeenCalled()
  })
})
