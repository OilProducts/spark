import { StrictMode, useState } from 'react'
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ModelChooser } from '../ModelChooser'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { useLlmProfiles } from '@/lib/useLlmProfiles'

vi.mock('@/lib/api/projectsApi', () => ({ fetchProjectChatModelsValidated: vi.fn() }))
vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: vi.fn() }))
const initial: ModelSettings = { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null }
const catalog: ProjectChatModelsResponse = {
    providers: { codex: { status: 'available', error: null } },
    models: [
        { provider: 'codex', id: 'discovered', display: 'Discovered', is_default: true, supported_reasoning_efforts: ['low', 'ultra'], default_reasoning_effort: 'low' },
        { provider: 'codex', id: 'other', display: 'Other', is_default: false, supported_reasoning_efforts: ['medium'] },
        { provider: 'claude-code', id: 'claude-discovered', display: 'Claude', is_default: true, supported_reasoning_efforts: [] },
    ],
}
function Editor({ value = initial, inherited, projectPath = '/project', onChange = vi.fn(), layout = 'fields', disabled = false, invalidModel = false }: {
    value?: ModelSettings; inherited?: ModelSettings; projectPath?: string | null; onChange?: (value: ModelSettings) => void; layout?: 'fields' | 'compact'; disabled?: boolean; invalidModel?: boolean
}) {
    const [draft, setDraft] = useState(value)
    return <ModelChooser value={draft} inherited={inherited} onChange={(next) => { setDraft(next); onChange(next) }}
        projectPath={projectPath} inheritLabel="Graph default" layout={layout} disabled={disabled} invalidModel={invalidModel} />
}
beforeEach(() => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', label: 'Team', provider: 'openai', configured: true, models: ['team-one', 'team-two'] }])
})
afterEach(async () => { cleanup(); await Promise.resolve(); vi.resetAllMocks() })

it('shows explicit, inherited and discovered defaults with effort, and unknown inheritance', async () => {
    const view = render(<Editor value={{ ...initial, model: 'discovered', reasoning_effort: 'ultra' }} />)
    await screen.findByRole('button', { name: 'Model: Discovered · Ultra' })
    view.rerender(<Editor key="inherited" value={{ ...initial, provider: null }} inherited={{ ...initial, model: 'other', reasoning_effort: 'medium' }} />)
    expect(screen.getByRole('button', { name: 'Model: Default: Other · Medium' })).toBeInTheDocument()
    view.rerender(<Editor key="discovery" />)
    expect(screen.getByRole('button', { name: 'Model: Default: Discovered · Low' })).toBeInTheDocument()
    view.rerender(<Editor key="unknown" projectPath={null} />)
    expect(screen.getByRole('button', { name: 'Model: Default: Graph default' })).toBeInTheDocument()
})

it('filters across provider, display, id and profile without emitting and commits atomic model/profile choices', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Default: Discovered/ }))
    const search = screen.getByRole('combobox', { name: 'Search models' })
    expect(search).toHaveFocus()
    for (const [query, result] of [['claude-code', 'Claude'], ['Other', 'Other'], ['team', 'team-one']]) {
        await user.clear(search); await user.type(search, query)
        expect(screen.getByRole('option', { name: result })).toBeInTheDocument()
        expect(screen.queryByRole('option', { name: 'Discovered' })).not.toBeInTheDocument()
    }
    expect(onChange).not.toHaveBeenCalled()
    await user.click(screen.getByRole('option', { name: 'team-two' }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-two' })
    expect(search).toBeVisible()
    await user.click(screen.getByRole('button', { name: 'High', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-two', reasoning_effort: 'high' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Model:/ }))
    await user.click(screen.getByRole('button', { name: /Use default/ }))
    expect(onChange).toHaveBeenLastCalledWith({ provider: null, llm_profile: null, model: null, reasoning_effort: null })
})

it('commits a custom model only on selection and retains unsupported saved effort', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor layout="compact" value={{ ...initial, model: 'discovered', reasoning_effort: 'future' }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Discovered · Future/ }))
    expect(screen.getByRole('button', { name: 'Future (custom)' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.queryByRole('button', { name: 'High', exact: true })).not.toBeInTheDocument()
    await user.type(screen.getByRole('combobox'), 'my-model')
    expect(onChange).not.toHaveBeenCalled()
    await user.click(screen.getByRole('option', { name: 'Use "my-model" as a custom model' }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, model: 'my-model', reasoning_effort: 'future' })
    await user.click(screen.getByRole('button', { name: 'High', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, model: 'my-model', reasoning_effort: 'high' })
})

it('uses highlighted metadata, selects with Enter and restores focus on Escape', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    const trigger = await screen.findByRole('button', { name: /Model: Default: Discovered/ })
    await user.click(trigger)
    await user.type(screen.getByRole('combobox'), 'Other')
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-activedescendant', screen.getByRole('option', { name: 'Other' }).id)
    expect(screen.getByRole('button', { name: 'Medium' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Ultra' })).not.toBeInTheDocument()
    await user.keyboard('{Enter}')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, model: 'other' })
    expect(trigger).toHaveFocus()
    await user.click(trigger)
    await user.keyboard('{ArrowUp}{Escape}')
    expect(trigger).toHaveFocus()
    expect(onChange).toHaveBeenCalledTimes(1)
})

it('uses provider metadata for profile models and respects known empty effort support', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true, models: ['discovered'], default_model: 'discovered' }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    await user.click(await screen.findByRole('button', { name: /Default: Discovered/ }))
    expect(screen.getByRole('button', { name: 'Ultra' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'High' })).not.toBeInTheDocument()
    await user.click(within(screen.getByRole('group', { name: 'Claude Code', exact: true })).getByRole('option'))
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button')).toHaveLength(1)
    expect(screen.getByRole('button', { name: 'Default', exact: true })).toBeInTheDocument()
})

it('does not resolve a profile without a default to an unrelated provider model', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true, models: ['profile-only'] }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalledOnce())
    await user.click(screen.getByRole('button', { name: 'Model: Default: Graph default' }))
    expect(within(screen.getByRole('group', { name: 'Codex / team' })).getByRole('option', { name: 'profile-only' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'High', exact: true })).toBeInTheDocument()
})

it('preserves disabled and invalid styling and message', async () => {
    const user = userEvent.setup()
    render(<Editor disabled invalidModel />)
    const trigger = screen.getByRole('button', { name: /Model:/ })
    expect(trigger).toBeDisabled()
    expect(trigger).toHaveAttribute('aria-invalid', 'true')
    expect(trigger).toHaveAccessibleDescription('Choose a compatible model for this provider or profile.')
    await user.click(trigger)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
})

it('shares loading, failed discovery and refresh across StrictMode choosers', async () => {
    const user = userEvent.setup()
    let reject!: (error: Error) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValue(new Promise((_, fail) => { reject = fail }))
    render(<StrictMode><Editor /><Editor layout="compact" /></StrictMode>)
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledExactlyOnceWith('/project')
    await user.click(screen.getAllByRole('button', { name: /Model:/ })[0])
    expect(screen.getByText('Loading models…')).toBeInTheDocument()
    await act(async () => reject(new Error('offline')))
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).getByText('Model discovery unavailable. Using suggestions.')).toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).getByRole('option', { name: 'gpt-5.5' })).toBeInTheDocument()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    act(() => window.dispatchEvent(new Event('spark:codex-connected')))
    await screen.findByRole('option', { name: 'Discovered' })
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(2)
})

it('keeps projects isolated and ignores pre-refresh responses', async () => {
    const user = userEvent.setup()
    let resolve!: (value: ProjectChatModelsResponse) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const view = render(<Editor projectPath="/one" />)
    await user.click(screen.getByRole('button', { name: /Model:/ }))
    act(() => window.dispatchEvent(new Event('spark:codex-connected')))
    await screen.findByRole('option', { name: 'Discovered' })
    await act(async () => resolve({ ...catalog, models: [] }))
    expect(screen.getByRole('option', { name: 'Discovered' })).toBeInTheDocument()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({ models: [], providers: { codex: { status: 'unavailable', error: 'offline' } } })
    view.rerender(<Editor projectPath="/two" />)
    await waitFor(() => expect(screen.queryByRole('option', { name: 'Discovered' })).not.toBeInTheDocument())
    expect(fetchProjectChatModelsValidated).toHaveBeenLastCalledWith('/two')
})

it('clears saved effort to the explicit provider default even with inherited settings', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    const saved = { ...initial, model: 'discovered', reasoning_effort: 'high' }
    render(<Editor value={saved} inherited={saved} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Discovered · High' }))
    await user.click(screen.getByRole('button', { name: 'Default', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({ ...saved, reasoning_effort: null })
    await user.click(screen.getByRole('button', { name: 'Model: Discovered · Low' }))
    await user.click(screen.getByRole('option', { name: 'Claude', exact: true }))
    expect(screen.getByRole('button', { name: 'Model: Claude · Default effort' })).toBeInTheDocument()
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'claude-code', model: 'claude-discovered' })
})

it('retains only Default and a custom saved effort when support is explicitly empty', async () => {
    const user = userEvent.setup()
    render(<Editor value={{ ...initial, provider: 'claude-code', model: 'claude-discovered', reasoning_effort: 'high' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Claude · High' }))
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button').map(button => button.textContent)).toEqual(['Default', 'High (custom)'])
})


it.each([
    { inherited: undefined, expected: initial, label: 'Default: Discovered · Low', effort: 'Ultra' },
    { inherited: { ...initial, model: 'other', reasoning_effort: 'medium' }, expected: { ...initial, model: 'other' }, label: 'Other · Medium', effort: 'Medium' },
    { inherited: { ...initial, provider: 'anthropic', model: 'parent-model', reasoning_effort: 'low' }, expected: { ...initial, provider: 'anthropic', model: 'parent-model' }, label: 'parent-model · High', effort: 'High' },
    { inherited: { ...initial, provider: null, llm_profile: 'team', model: 'team-two', reasoning_effort: 'low' }, expected: { ...initial, provider: null, llm_profile: 'team', model: 'team-two' }, label: 'team-two · High', effort: 'High' },
])('commits the displayed inherited selection atomically after clearing: $label', async ({ inherited, expected, effort }) => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor value={{ ...initial, model: 'discovered', reasoning_effort: 'high' }} inherited={inherited} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Discovered · High' }))
    await user.click(screen.getByRole('button', { name: /Use default/ }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ provider: null, llm_profile: null, model: null, reasoning_effort: null })
    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(screen.getByRole('button', { name: effort, exact: true }))
    expect(onChange).toHaveBeenCalledTimes(2)
    expect(onChange).toHaveBeenLastCalledWith({ ...expected, reasoning_effort: effort.toLowerCase() })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
})

it('keeps a cleared selection inherited when choosing Default effort', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    const cleared = { provider: null, llm_profile: null, model: null, reasoning_effort: null }
    render(<Editor value={cleared} inherited={{ ...initial, provider: 'anthropic', model: 'parent-model', reasoning_effort: 'high' }} onChange={onChange} />)
    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(screen.getByRole('button', { name: 'Default', exact: true }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith(cleared)
    expect(screen.getByRole('button', { name: 'Model: Default: parent-model · High' })).toBeInTheDocument()
})
