import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { saveFlowContent, sendPendingFlowSaves } from '@/lib/flowPersistence'
import { useStore } from '@/store'

vi.mock('@/lib/flowPersistence', () => ({
  saveFlowContent: vi.fn(),
  registerPendingFlowSave: () => () => undefined,
  sendPendingFlowSaves: vi.fn(() => false),
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
      null,
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

  it('sends a pending edit on reload, and warns while a save is still in flight', () => {
    useStore.setState({ activeProjectPath: '/tmp/project-a', editorMode: 'structured' })
    render(<SchedulerHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Count 0' }))

    vi.mocked(sendPendingFlowSaves).mockReturnValueOnce(true)
    const unload = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(unload)
    expect(saveFlowContent).toHaveBeenCalledTimes(1)
    expect(unload.defaultPrevented).toBe(true)

    const idle = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(idle)
    expect(idle.defaultPrevented).toBe(false)
  })
})
