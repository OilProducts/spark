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
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', label: 'Team', provider: 'openai', configured: true, models: ['team-one', 'team-two'], reasoning_efforts: ['high'] }])
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
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button')).toHaveLength(1)
    expect(screen.getByRole('button', { name: 'Model: my-model · Future' })).toBeInTheDocument()
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

it('uses declared profile efforts and respects known empty effort support', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true, models: ['discovered'], default_model: 'discovered', reasoning_efforts: ['ultra'] }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    await user.click(await screen.findByRole('button', { name: /Default: discovered/ }))
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
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button')).toHaveLength(1)
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
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).getByText('Model discovery unavailable. offline')).toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).queryByRole('option')).not.toBeInTheDocument()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    act(() => window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section: 'codex' } } })))
    await screen.findByRole('option', { name: 'Discovered' })
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(2)
})

it('keeps projects isolated and ignores pre-refresh responses', async () => {
    const user = userEvent.setup()
    let resolve!: (value: ProjectChatModelsResponse) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const view = render(<Editor projectPath="/one" />)
    await user.click(screen.getByRole('button', { name: /Model:/ }))
    act(() => window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section: 'codex' } } })))
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

it('offers only Default when support is empty while retaining the stored effort', async () => {
    const user = userEvent.setup()
    render(<Editor value={{ ...initial, provider: 'claude-code', model: 'claude-discovered', reasoning_effort: 'high' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Claude · High' }))
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button').map(button => button.textContent)).toEqual(['Default'])
})


it('locks effort while inheriting and sets it only with an explicitly chosen model', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    const cleared = { provider: null, llm_profile: null, model: null, reasoning_effort: null }
    render(<Editor value={cleared} inherited={{ ...initial, provider: 'anthropic', model: 'parent-model', reasoning_effort: 'high' }} onChange={onChange} />)
    await user.click(screen.getByRole('button', { name: 'Model: Default: parent-model · High' }))
    const effort = within(screen.getByRole('group', { name: 'Reasoning effort' }))
    expect(effort.getByText('Effort follows the default. Choose a model to set it.')).toBeInTheDocument()
    expect(effort.getAllByRole('button').every(button => (button as HTMLButtonElement).disabled)).toBe(true)
    await user.keyboard('{ArrowDown}')
    const first = effort.getAllByRole('button')[1]
    expect(first).toBeEnabled()
    await user.click(first)
    const chosen = onChange.mock.lastCall![0]
    expect(chosen).toMatchObject({ reasoning_effort: first.textContent!.toLowerCase(), model: expect.any(String) })
    expect(chosen.provider ?? chosen.llm_profile).toBeTruthy()
    expect(chosen.model).not.toBe('parent-model')
})

it('uses model levels and provider fallbacks without replacing known empty levels', async () => {
    const user = userEvent.setup()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        ...catalog,
        provider_reasoning_efforts: { openai: ['none', 'minimal', 'high'] },
        models: [...catalog.models,
            { provider: 'openai', id: 'known', display: 'Known', is_default: false, supported_reasoning_efforts: ['none', 'minimal'], default_reasoning_effort: 'none' },
            { provider: 'openai', id: 'empty', display: 'Empty', is_default: false, supported_reasoning_efforts: [] },
        ],
    })
    render(<Editor value={{ ...initial, provider: 'openai', model: 'known' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Known · None' }))
    const buttons = () => within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button').map(button => button.textContent)
    expect(buttons()).toEqual(['Default', 'None', 'Minimal'])
    await user.click(screen.getByRole('option', { name: 'Empty', exact: true }))
    expect(buttons()).toEqual(['Default'])
    expect(screen.queryByText(/unverified/)).not.toBeInTheDocument()
    await user.type(screen.getByRole('combobox'), 'unlisted')
    await user.click(screen.getByRole('option', { name: 'Use "unlisted" as a custom model' }))
    expect(buttons()).toEqual(['Default', 'None', 'Minimal', 'High'])
    expect(screen.getByText('Provider levels; unverified for this model.')).toBeInTheDocument()
})

it('keeps declarations separate for profiles sharing a provider and model', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([
        { id: 'first', provider: 'openai_compatible', configured: true, models: ['shared'], reasoning_efforts: ['minimal'] },
        { id: 'second', provider: 'openai_compatible', configured: true, models: ['shared'] },
    ])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'first', model: 'shared' }} />)
    await user.click(screen.getByRole('button', { name: /Model:/ }))
    expect(screen.getByRole('button', { name: 'Minimal', exact: true })).toBeInTheDocument()
    await user.click(within(screen.getByRole('group', { name: 'openai_compatible / second' })).getByRole('option'))
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button')).toHaveLength(1)
})

it('lists only discovered providers, retains unavailable errors, and marks fallback efforts unverified', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([])
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: { codex: { status: 'unavailable', error: 'CLI missing' }, openai: { status: 'unavailable', error: 'HTTP 401' }, gemini: { status: 'available', error: null } },
        models: [{ provider: 'gemini', id: 'new', display: 'New', is_default: false, supported_reasoning_efforts: ['high'], reasoning_unverified: true }],
    })
    const user = userEvent.setup()
    render(<Editor value={{ ...initial, provider: 'gemini', model: 'new' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: New · Default effort' }))
    expect(screen.queryByRole('group', { name: 'anthropic' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'openrouter' })).not.toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: 'openai', exact: true })).getByRole('status')).toHaveTextContent('HTTP 401')
    expect(within(screen.getByRole('group', { name: 'openai', exact: true })).queryByRole('option')).not.toBeInTheDocument()
    expect(screen.getAllByRole('option')).toHaveLength(1)
    expect(screen.getByText('Provider levels; unverified for this model.')).toBeInTheDocument()
})

it('fetches once per project for each affected settings event and ignores unrelated saves', async () => {
    render(<StrictMode><Editor /><Editor layout="compact" /></StrictMode>)
    await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(1))
    for (const [index, section] of ['providers', 'llm_profiles', 'agents', 'codex'].entries()) {
        act(() => window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section } } })))
        await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(index + 2))
    }
    act(() => window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section: 'preferences' } } })))
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(5)
})

it('gates native controls by capability, locks inheritance, and saves budgets and summaries', async () => {
    const user = userEvent.setup()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({ ...catalog, models: [
        { ...catalog.models[0], supported_thinking: ['adaptive', 'budget'], supported_reasoning_modes: ['standard', 'pro'], supported_reasoning_summaries: ['auto', 'detailed'] },
        catalog.models[1],
    ] })
    const onChange = vi.fn()
    render(<Editor value={{ ...initial, provider: null }} inherited={{ ...initial, model: 'discovered' }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Default: Discovered/ }))
    const thinking = screen.getByRole('group', { name: 'Thinking' })
    expect(within(thinking).queryByRole('button', { name: 'Off' })).not.toBeInTheDocument()
    expect(within(thinking).getByRole('button', { name: 'Budget' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name: 'Mode' })).getByRole('button', { name: 'Pro' })).toBeDisabled()
    await user.click(screen.getByRole('option', { name: 'Discovered' }))
    await user.click(within(thinking).getByRole('button', { name: 'Budget' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: 'budget', thinking_budget_tokens: 1024 }))
    const budget = screen.getByRole('spinbutton', { name: 'Thinking budget tokens' })
    expect(budget).toHaveAttribute('min', '1024')
    await user.clear(budget)
    await user.type(budget, '2048')
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking_budget_tokens: 2048 }))
    await user.click(within(screen.getByRole('group', { name: 'Mode' })).getByRole('button', { name: 'Pro' }))
    await user.click(within(screen.getByRole('group', { name: 'Summary' })).getByRole('button', { name: 'Detailed' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: 'budget', thinking_budget_tokens: 2048, reasoning_mode: 'pro', reasoning_summary: 'detailed' }))
    await user.click(within(thinking).getByRole('button', { name: 'Adaptive' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: 'adaptive', thinking_budget_tokens: null }))
    await user.click(screen.getByRole('option', { name: 'Other' }))
    expect(screen.queryByRole('group', { name: 'Thinking' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Summary' })).not.toBeInTheDocument()
})
