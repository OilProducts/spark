import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchRuntimeSettings, saveRuntimeSettings, type RuntimeSettingsView } from '@/lib/api/settingsApi'
import { ApiHttpError } from '@/lib/api/shared'
import { requestNavigation } from '@/state/workspaceSlice'
import { RuntimeSettingsEditor } from '../RuntimeSettingsEditor'

vi.mock('@/lib/api/settingsApi', () => ({ fetchRuntimeSettings: vi.fn(), saveRuntimeSettings: vi.fn() }))
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.resetAllMocks() })

it('retains edits across late reads and failed saves', async () => {
    const view: RuntimeSettingsView = {
        scope: 'workspace', revision: 'revision-1', restart_fields: ['runs_dir'],
        stored: { runs_dir: '/saved', ui_dir: null, project_roots: [] },
        effective: { runs_dir: '/effective', ui_dir: null, project_roots: [] },
    }
    let first!: (value: RuntimeSettingsView) => void
    let late!: (value: RuntimeSettingsView) => void
    vi.mocked(fetchRuntimeSettings).mockImplementationOnce(() => new Promise((resolve) => { first = resolve }))
        .mockImplementationOnce(() => new Promise((resolve) => { late = resolve }))
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await act(async () => first(view))
    const user = userEvent.setup()
    const input = screen.getByLabelText('Runs directory')
    await user.clear(input)
    await user.type(input, '/draft')
    await act(async () => late({ ...view, revision: 'late-revision' }))
    expect(input).toHaveValue('/draft')
    vi.mocked(saveRuntimeSettings).mockRejectedValueOnce(new Error('Concurrent update.'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Concurrent update.')
    expect(input).toHaveValue('/draft')
    expect(saveRuntimeSettings).toHaveBeenCalledWith('revision-1', { ...view.stored, runs_dir: '/draft' })
})

it('blocks navigation while a runtime save is pending and allows it once saved', async () => {
    const saved = { runs_dir: '/saved', ui_dir: null, project_roots: [] }
    const view: RuntimeSettingsView = {
        scope: 'workspace', revision: 'revision-1', restart_fields: ['runs_dir'], stored: saved, effective: saved,
    }
    vi.mocked(fetchRuntimeSettings).mockResolvedValue(view)
    let finishSave!: (value: RuntimeSettingsView) => void
    vi.mocked(saveRuntimeSettings).mockImplementation(() => new Promise((resolve) => { finishSave = resolve }))
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)

    const user = userEvent.setup()
    const field = await screen.findByLabelText('Runs directory')
    await user.clear(field)
    await user.type(field, '/draft')
    const save = screen.getByRole('button', { name: /^Save/ })
    await user.click(save)
    expect(save).toBeDisabled()

    const proceed = vi.fn()
    act(() => requestNavigation(proceed))
    expect(proceed).not.toHaveBeenCalled()
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()

    await act(async () => finishSave({ ...view, revision: 'revision-2', stored: { ...saved, runs_dir: '/draft' } }))
    act(() => requestNavigation(proceed))
    expect(proceed).toHaveBeenCalledOnce()
})

it('describes an unset runs directory with its default placeholder and only shows provenance when overridden', async () => {
    const unset = { runs_dir: null, ui_dir: null, project_roots: [] }
    vi.mocked(fetchRuntimeSettings).mockResolvedValue({
        scope: 'workspace', revision: 'revision-1', restart_fields: ['runs_dir'], stored: unset, effective: unset,
        sources: { runs_dir: 'built-in default', ui_dir: 'environment: SPARK_UI_DIR', project_roots: 'built-in default' },
    })
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)
    expect(await screen.findByLabelText('Runs directory')).toHaveAttribute('placeholder', "Each project's private storage")
    expect(screen.getByLabelText('UI directory')).toHaveAttribute('placeholder', 'Not configured')
    expect(screen.queryByText(/In use: Each project's private storage/)).not.toBeInTheDocument()
    expect(screen.getByText(/In use: Not configured · environment: SPARK_UI_DIR/)).toBeInTheDocument()
})

it('sends bare ~, drive and UNC project roots to the backend as typed', async () => {
    const unset = { runs_dir: null, ui_dir: null, project_roots: [] }
    const view: RuntimeSettingsView = { scope: 'workspace', revision: 'revision-1', restart_fields: ['project_roots'], stored: unset, effective: unset }
    vi.mocked(fetchRuntimeSettings).mockResolvedValue(view)
    vi.mocked(saveRuntimeSettings).mockImplementation(async (_revision, value) => ({ ...view, revision: 'revision-2', stored: value }))
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)

    const roots = ['~', 'C:\\projects', '\\\\server\\share\\projects']
    fireEvent.change(await screen.findByLabelText(/^Project roots/), { target: { value: roots.join('\n') } })
    const save = screen.getByRole('button', { name: /^Save/ })
    expect(save).toBeEnabled()
    await userEvent.setup().click(save)
    expect(saveRuntimeSettings).toHaveBeenCalledWith('revision-1', { ...unset, project_roots: roots })
    expect(await screen.findByText(/^Saved/)).toBeInTheDocument()
})

it('shows the backend rejecting a project root in the editor', async () => {
    const unset = { runs_dir: null, ui_dir: null, project_roots: [] }
    vi.mocked(fetchRuntimeSettings).mockResolvedValue({ scope: 'workspace', revision: 'revision-1', restart_fields: ['project_roots'], stored: unset, effective: unset })
    vi.mocked(saveRuntimeSettings).mockRejectedValueOnce(new ApiHttpError('/workspace/api/settings', 422, 'project root must be an absolute path: C:\\projects'))
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)

    const field = await screen.findByLabelText(/^Project roots/)
    fireEvent.change(field, { target: { value: 'C:\\projects' } })
    await userEvent.setup().click(screen.getByRole('button', { name: /^Save/ }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Project root must be an absolute path: C:\\projects')
    expect(field).toHaveValue('C:\\projects')
})
