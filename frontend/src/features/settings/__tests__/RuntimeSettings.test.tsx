import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchRuntimeSettings, saveRuntimeSettings, type RuntimeSettingsView } from '@/lib/api/settingsApi'
import { RuntimeSettingsEditor } from '../RuntimeSettingsEditor'

vi.mock('@/lib/api/settingsApi', () => ({ fetchRuntimeSettings: vi.fn(), saveRuntimeSettings: vi.fn() }))
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.resetAllMocks() })

it('retains edits across late reads and failed saves', async () => {
    const view: RuntimeSettingsView = {
        scope: 'workspace', revision: 'revision-1', restart_fields: ['flows_dir'],
        stored: { flows_dir: '/saved', runs_dir: null, ui_dir: null, project_roots: [] },
        effective: { flows_dir: '/effective', runs_dir: null, ui_dir: null, project_roots: [] },
    }
    let first!: (value: RuntimeSettingsView) => void
    let late!: (value: RuntimeSettingsView) => void
    vi.mocked(fetchRuntimeSettings).mockImplementationOnce(() => new Promise((resolve) => { first = resolve }))
        .mockImplementationOnce(() => new Promise((resolve) => { late = resolve }))
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await act(async () => first(view))
    const user = userEvent.setup()
    const input = screen.getByLabelText('Flows directory')
    await user.clear(input)
    await user.type(input, '/draft')
    await act(async () => late({ ...view, revision: 'late-revision' }))
    expect(input).toHaveValue('/draft')
    vi.mocked(saveRuntimeSettings).mockRejectedValueOnce(new Error('Concurrent update.'))
    await user.click(screen.getByRole('button', { name: 'Save runtime settings' }))
    await screen.findByText('Concurrent update.')
    expect(input).toHaveValue('/draft')
    expect(saveRuntimeSettings).toHaveBeenCalledWith('revision-1', { ...view.stored, flows_dir: '/draft' })
})
