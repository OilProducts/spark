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
const cleared: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }
const catalog: ProjectChatModelsResponse = {
    providers: { codex: { status: 'available', error: null, label: 'Codex' }, 'claude-code': { status: 'available', error: null, label: 'Claude Code' } },
    models: [
        { provider: 'codex', id: 'discovered', display: 'Discovered', is_default: true, supported_reasoning_efforts: ['low', 'ultra'], default_reasoning_effort: 'low' },
        { provider: 'codex', id: 'other', display: 'Other', is_default: false, supported_reasoning_efforts: ['medium'] },
        { provider: 'claude-code', id: 'claude-discovered', display: 'Claude', is_default: true, supported_reasoning_efforts: [] },
    ],
}
function Editor({ value = initial, inherited, projectPath = '/project', onChange = vi.fn(), layout = 'fields', disabled = false, invalidModel = false, defaultPrefix, title }: {
    value?: ModelSettings; inherited?: ModelSettings; projectPath?: string | null; onChange?: (value: ModelSettings) => void; layout?: 'fields' | 'compact'; disabled?: boolean; invalidModel?: boolean; defaultPrefix?: string; title?: string
}) {
    const [draft, setDraft] = useState(value)
    return <ModelChooser value={draft} inherited={inherited} onChange={(next) => { setDraft(next); onChange(next) }}
        projectPath={projectPath} inheritLabel="Graph default" layout={layout} disabled={disabled} invalidModel={invalidModel}
        defaultPrefix={defaultPrefix} title={title} />
}
beforeEach(() => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', label: 'Team', provider: 'openai', configured: true, models: ['team-one', 'team-two'], reasoning_efforts: ['high'] }])
})
afterEach(async () => { cleanup(); await Promise.resolve(); vi.resetAllMocks() })

it('shows explicit, inherited and discovered defaults across split Model and Effort triggers, plus unknown inheritance', async () => {
    const view = render(<Editor value={{ ...initial, model: 'discovered', reasoning_effort: 'ultra' }} />)
    const modelBtn = await screen.findByRole('button', { name: 'Model: Discovered, Codex' })
    expect(modelBtn).toHaveTextContent('Discovered')
    expect(modelBtn).toHaveTextContent('Codex')
    expect(screen.getByRole('button', { name: 'Reasoning effort: Ultra' })).toHaveTextContent('Ultra')

    view.rerender(<Editor key="inherited" layout="compact" value={cleared} inherited={{ ...initial, model: 'other', reasoning_effort: 'medium' }} defaultPrefix="Project default" title="Change it in project settings." />)
    const compactInherited = screen.getByRole('button', { name: 'Model: Other, Codex, Project default' })
    expect(compactInherited).toHaveTextContent('Other')
    expect(compactInherited).toHaveTextContent('Codex')
    expect(compactInherited).not.toHaveTextContent(/Project default:|Default effort/)
    expect(compactInherited).toHaveAttribute('data-inherited', 'true')
    expect(compactInherited).toHaveAttribute('title', 'Project default: Other (Codex). Change it in project settings.')
    expect(screen.getByRole('button', { name: 'Reasoning effort: Medium (default)' })).toHaveTextContent('Medium')

    view.rerender(<Editor key="discovery" />)
    expect(screen.getByRole('button', { name: 'Model: Discovered, Codex, Default' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reasoning effort: Low (default)' })).toBeInTheDocument()

    view.rerender(<Editor key="unknown" value={cleared} projectPath={null} />)
    expect(screen.getByRole('button', { name: 'Model: Graph default, Default' })).toBeInTheDocument()
    const lockedEffort = screen.getByRole('button', { name: 'Reasoning effort: unavailable' })
    expect(lockedEffort).toBeDisabled()
    expect(lockedEffort).toHaveAccessibleDescription('Choose a model to set reasoning effort.')
})

it('filters across provider, display, id and profile without emitting and commits atomic model/profile choices in 1 click', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    const search = screen.getByRole('combobox', { name: 'Search models' })
    expect(search).toHaveFocus()
    for (const [query, result] of [['claude-code', 'Claude'], ['Other', 'Other'], ['team', 'team-one']]) {
        await user.clear(search); await user.type(search, query)
        expect(screen.getByRole('option', { name: result })).toBeInTheDocument()
        expect(screen.queryByRole('option', { name: 'Discovered' })).not.toBeInTheDocument()
    }
    expect(onChange).not.toHaveBeenCalled()
    // 1-click commit closes the Model popover immediately.
    await user.click(screen.getByRole('option', { name: 'team-two' }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-two' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    // Effort has its own adjacent trigger and closes immediately on selection.
    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
    await user.click(screen.getByRole('button', { name: 'High', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-two', reasoning_effort: 'high' })
    expect(screen.queryByRole('group', { name: 'Reasoning effort' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(screen.getByRole('button', { name: /Use default/ }))
    expect(onChange).toHaveBeenLastCalledWith(cleared)
})

it('commits a custom model only on selection and retains unsupported saved effort', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor layout="compact" value={{ ...initial, model: 'discovered', reasoning_effort: 'future' }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: 'Reasoning effort: Future' }))
    expect(screen.getByRole('button', { name: 'Future (custom)' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.queryByRole('button', { name: 'High', exact: true })).not.toBeInTheDocument()
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: 'Model: Discovered, Codex' }))
    await user.type(screen.getByRole('combobox'), 'my-model')
    expect(onChange).not.toHaveBeenCalled()
    await user.click(screen.getByRole('option', { name: 'Use "my-model" as a custom model' }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, model: 'my-model', reasoning_effort: 'future' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Model: my-model, Codex' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Reasoning effort: Future' }))
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button')).toHaveLength(1)
})

it('pre-highlights first search match so open -> type -> Enter commits and closes, and preserves Arrow wrap and Escape focus', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    const trigger = await screen.findByRole('button', { name: /Model: Discovered/ })
    await user.click(trigger)
    const search = screen.getByRole('combobox', { name: 'Search models' })
    await user.type(search, 'Other')
    // Pre-highlighted immediately without pressing ArrowDown.
    expect(search).toHaveAttribute('aria-activedescendant', screen.getByRole('option', { name: 'Other' }).id)
    await user.keyboard('{Enter}')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, model: 'other' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()

    // Effort trigger now reflects the newly selected model's supported efforts.
    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
    expect(screen.getByRole('button', { name: 'Medium' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Ultra' })).not.toBeInTheDocument()
    await user.keyboard('{Escape}')

    // ArrowUp wraps to the last item and Escape restores focus to the Model trigger without emitting.
    await user.click(trigger)
    await user.keyboard('{ArrowUp}')
    const options = screen.getAllByRole('option')
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-activedescendant', options[options.length - 1].id)
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-activedescendant', options[0].id)
    await user.keyboard('{Escape}')
    expect(trigger).toHaveFocus()
    expect(onChange).toHaveBeenCalledTimes(1)
})

it('uses declared profile efforts and hides Effort trigger when a known model supports no efforts', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true, models: ['discovered'], default_model: 'discovered', reasoning_efforts: ['ultra'] }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    await user.click(await screen.findByRole('button', { name: /^Reasoning effort:/ }))
    expect(screen.getByRole('button', { name: 'Ultra' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'High' })).not.toBeInTheDocument()
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(within(screen.getByRole('group', { name: 'Claude Code', exact: true })).getByRole('option'))
    expect(screen.queryByRole('button', { name: /^Reasoning effort:/ })).not.toBeInTheDocument()
})

it('does not resolve a profile without a default to an unrelated provider model', async () => {
    const user = userEvent.setup()
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true, models: ['profile-only'] }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalledOnce())
    await user.click(screen.getByRole('button', { name: 'Model: Graph default, Default' }))
    expect(within(screen.getByRole('group', { name: 'Codex / team' })).getByRole('option', { name: 'profile-only' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Reasoning effort:/ })).not.toBeInTheDocument()
})

it('preserves disabled and invalid styling and message', async () => {
    const user = userEvent.setup()
    render(<Editor disabled invalidModel />)
    const trigger = screen.getByRole('button', { name: /^Model:/ })
    expect(trigger).toBeDisabled()
    expect(trigger).toHaveAttribute('aria-invalid', 'true')
    expect(trigger).toHaveAccessibleDescription('Choose a compatible model for this provider or profile.')
    await user.click(trigger)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
})

it('shares loading, failed discovery and refresh across StrictMode choosers and collapses unavailable errors into the footer', async () => {
    const user = userEvent.setup()
    let reject!: (error: Error) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValue(new Promise((_, fail) => { reject = fail }))
    render(<StrictMode><Editor /><Editor layout="compact" /></StrictMode>)
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledExactlyOnceWith('/project')
    await user.click(screen.getAllByRole('button', { name: /^Model:/ })[0])
    expect(screen.getByText('Loading models…')).toBeInTheDocument()
    await act(async () => reject(new Error('offline')))
    // Unavailable provider is collapsed into the muted footer disclosure, not inline in the model list.
    expect(screen.queryByRole('group', { name: 'Codex', exact: true })).not.toBeInTheDocument()
    const unavailableToggle = screen.getByRole('button', { name: '1 unavailable provider' })
    expect(unavailableToggle).toHaveAttribute('aria-expanded', 'false')
    await user.click(unavailableToggle)
    expect(unavailableToggle).toHaveAttribute('aria-expanded', 'true')
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).getByRole('status')).toHaveTextContent('Model discovery unavailable. offline')
    expect(within(screen.getByRole('group', { name: 'Codex', exact: true })).queryByRole('option')).not.toBeInTheDocument()

    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    act(() => window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section: 'codex' } } })))
    await screen.findByRole('option', { name: 'Discovered' })
    // Mount, opening, then the settings event.
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(3)
})

it('keeps projects isolated and ignores pre-refresh responses', async () => {
    const user = userEvent.setup()
    let resolve!: (value: ProjectChatModelsResponse) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const view = render(<Editor projectPath="/one" />)
    await user.click(screen.getByRole('button', { name: /^Model:/ }))
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
    await user.click(await screen.findByRole('button', { name: 'Reasoning effort: High' }))
    await user.click(screen.getByRole('button', { name: 'Default', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({ ...saved, reasoning_effort: null })
    expect(screen.getByRole('button', { name: 'Reasoning effort: Low (default)' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Model: Discovered, Codex' }))
    await user.click(screen.getByRole('option', { name: 'Claude', exact: true }))
    expect(screen.getByRole('button', { name: 'Model: Claude, Claude Code' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Reasoning effort:/ })).not.toBeInTheDocument()
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'claude-code', model: 'claude-discovered' })
})

it('unlocks effort while inheriting when effective model is resolved and atomically pins model + effort', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor value={cleared} inherited={{ ...initial, model: 'discovered', reasoning_effort: null }} onChange={onChange} />)
    const effortTrigger = await screen.findByRole('button', { name: 'Reasoning effort: Low (default)' })
    expect(effortTrigger).toBeEnabled()
    await user.click(effortTrigger)
    await user.click(screen.getByRole('button', { name: 'Ultra', exact: true }))
    expect(onChange).toHaveBeenCalledExactlyOnceWith({
        provider: 'codex', llm_profile: null, model: 'discovered', reasoning_effort: 'ultra',
    })
    expect(screen.queryByRole('group', { name: 'Reasoning effort' })).not.toBeInTheDocument()
})

it('keeps effort disabled with a clear hint when no effective model is known', async () => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: { codex: { status: 'unavailable', error: 'CLI missing', label: 'Codex' } },
        models: [],
    })
    render(<Editor value={cleared} inherited={{ ...initial, model: null }} />)
    await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalled())
    const effortTrigger = screen.getByRole('button', { name: 'Reasoning effort: unavailable' })
    expect(effortTrigger).toBeDisabled()
    expect(effortTrigger).toHaveAttribute('title', 'Choose a model to set reasoning effort.')
    expect(effortTrigger).toHaveAccessibleDescription('Choose a model to set reasoning effort.')
})

it('keeps a saved effort on a model with no levels and uses fallbacks only for unlisted models', async () => {
    const user = userEvent.setup()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        ...catalog,
        provider_reasoning_efforts: { openai: ['none', 'minimal', 'high'] },
        models: [...catalog.models,
            { provider: 'openai', id: 'known', display: 'Known', is_default: false, supported_reasoning_efforts: ['none', 'minimal'], default_reasoning_effort: 'none' },
            { provider: 'openai', id: 'empty', display: 'Empty', is_default: false, supported_reasoning_efforts: [] },
        ],
    })
    render(<Editor value={{ ...initial, provider: 'openai', model: 'empty', reasoning_effort: 'high' }} />)
    await user.click(await screen.findByRole('button', { name: 'Reasoning effort: High' }))
    const buttons = () => within(screen.getByRole('group', { name: 'Reasoning effort' })).getAllByRole('button').map(button => button.textContent)
    expect(buttons()).toEqual(['Default'])
    expect(screen.queryByText(/unverified/)).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Default', exact: true }))
    expect(screen.queryByRole('button', { name: /^Reasoning effort:/ })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Model: Empty, openai' }))
    await user.click(screen.getByRole('option', { name: 'Known', exact: true }))
    await user.click(screen.getByRole('button', { name: 'Reasoning effort: None (default)' }))
    expect(buttons()).toEqual(['Default', 'None', 'Minimal'])
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: 'Model: Known, openai' }))
    await user.type(screen.getByRole('combobox'), 'unlisted')
    await user.click(screen.getByRole('option', { name: 'Use "unlisted" as a custom model' }))
    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
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
    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
    expect(screen.getByRole('button', { name: 'Minimal', exact: true })).toBeInTheDocument()
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(within(screen.getByRole('group', { name: 'openai_compatible / second' })).getByRole('option'))
    expect(screen.queryByRole('button', { name: /^Reasoning effort:/ })).not.toBeInTheDocument()
})

it('lists only discovered providers, collapses unavailable errors into expandable footer, and marks fallback efforts unverified', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([])
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: { codex: { status: 'unavailable', error: 'CLI missing' }, openai: { status: 'unavailable', error: 'HTTP 401' }, gemini: { status: 'available', error: null } },
        models: [{ provider: 'gemini', id: 'new', display: 'New', is_default: false, supported_reasoning_efforts: ['high'], reasoning_unverified: true }],
    })
    const user = userEvent.setup()
    render(<Editor value={{ ...initial, provider: 'gemini', model: 'new' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: New, gemini' }))
    expect(screen.queryByRole('group', { name: 'anthropic' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'openrouter' })).not.toBeInTheDocument()
    expect(screen.getAllByRole('option')).toHaveLength(1)
    expect(screen.queryByRole('group', { name: 'openai', exact: true })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '2 unavailable providers' }))
    expect(within(screen.getByRole('group', { name: 'openai', exact: true })).getByRole('status')).toHaveTextContent('HTTP 401')
    expect(within(screen.getByRole('group', { name: 'codex', exact: true })).getByRole('status')).toHaveTextContent('CLI missing')
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
    expect(screen.getByText('Provider levels; unverified for this model.')).toBeInTheDocument()
})

it('persists pinned favorites and recent models in localStorage, shows them at top when search is empty, and filters unavailable entries', async () => {
    const user = userEvent.setup()
    const view = render(<Editor />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    expect(screen.queryByRole('group', { name: 'Pinned' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Recent' })).not.toBeInTheDocument()

    // Pin Claude without closing the popover; Pinned appears at top.
    await user.click(screen.getByRole('button', { name: 'Pin Claude (Claude Code)' }))
    const pinnedGroup = screen.getByRole('group', { name: 'Pinned' })
    expect(within(pinnedGroup).getByRole('option', { name: 'Claude, Claude Code' })).toBeInTheDocument()

    // Select Other (1-click commits, closes, and records as recent).
    await user.click(screen.getByRole('option', { name: 'Other', exact: true }))
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    // Remount to simulate reload: both Pinned and Recent persist from localStorage.
    view.unmount()
    const reloaded = render(<Editor value={{ ...initial, model: 'other' }} />)
    await user.click(await screen.findByRole('button', { name: 'Model: Other, Codex' }))
    expect(within(screen.getByRole('group', { name: 'Pinned' })).getByRole('option', { name: 'Claude, Claude Code' })).toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: 'Recent' })).getByRole('option', { name: 'Other, Codex' })).toBeInTheDocument()

    // Typing search hides Pinned/Recent sections.
    await user.type(screen.getByRole('combobox', { name: 'Search models' }), 'Discovered')
    expect(screen.queryByRole('group', { name: 'Pinned' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Recent' })).not.toBeInTheDocument()

    // Unpinning removes from Pinned.
    await user.clear(screen.getByRole('combobox', { name: 'Search models' }))
    await user.click(within(screen.getByRole('group', { name: 'Pinned' })).getByRole('button', { name: 'Unpin Claude (Claude Code)' }))
    expect(screen.queryByRole('group', { name: 'Pinned' })).not.toBeInTheDocument()

    // Unavailable / removed models in localStorage are omitted.
    reloaded.unmount()
    localStorage.setItem('spark.model_chooser.pinned', JSON.stringify([
        { provider: 'codex', llm_profile: null, model: 'removed-model' },
        { provider: 'codex', llm_profile: null, model: 'discovered' },
    ]))
    localStorage.setItem('spark.model_chooser.recents', JSON.stringify([
        { provider: 'missing-provider', llm_profile: null, model: 'ghost' },
    ]))
    render(<Editor />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    const pinnedAfterFilter = screen.getByRole('group', { name: 'Pinned' })
    expect(within(pinnedAfterFilter).getByRole('option', { name: 'Discovered, Codex' })).toBeInTheDocument()
    expect(within(pinnedAfterFilter).queryByRole('option', { name: /removed-model/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Recent' })).not.toBeInTheDocument()
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

it('shows a harness as checking until its check ends, then its models without a reload', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([])
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({ ...catalog, providers: { ...catalog.providers, 'acp:slow': { status: 'checking', error: null, label: 'Slow' } } })
    const user = userEvent.setup()
    render(<Editor />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    expect(within(screen.getByRole('group', { name: 'Slow' })).getByRole('status')).toHaveTextContent('Checking…')

    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: { ...catalog.providers, 'acp:slow': { status: 'available', error: null, label: 'Slow', setup_status: 'ready' } },
        models: [...catalog.models, { provider: 'acp:slow', id: 'quick', display: 'Quick', is_default: true, supported_reasoning_efforts: [] }],
    })
    const fetched = vi.mocked(fetchProjectChatModelsValidated).mock.calls.length
    act(() => window.dispatchEvent(new Event('spark:harness-checked')))
    expect(screen.queryByText('Loading models…')).not.toBeInTheDocument()
    expect(await within(screen.getByRole('group', { name: 'Slow' })).findByRole('option', { name: 'Quick' })).toBeInTheDocument()
    expect(screen.queryByText('Checking…')).not.toBeInTheDocument()
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(fetched + 1)
})

it('tucks native controls behind More options, gates by capability, validates budget >= 1024, and surfaces active summary', async () => {
    const user = userEvent.setup()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({ ...catalog, models: [
        { ...catalog.models[0], supported_thinking: ['adaptive', 'budget'], supported_reasoning_modes: ['standard', 'pro'], supported_reasoning_summaries: ['auto', 'detailed'] },
        catalog.models[1],
    ] })
    const onChange = vi.fn()
    render(<Editor value={cleared} inherited={{ ...initial, model: 'discovered' }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    // Secondary controls are collapsed by default behind More options.
    expect(screen.queryByRole('group', { name: 'Thinking' })).not.toBeInTheDocument()
    const moreOptions = screen.getByRole('button', { name: /More options/ })
    expect(moreOptions).toHaveAttribute('aria-expanded', 'false')
    await user.click(moreOptions)

    const thinking = screen.getByRole('group', { name: 'Thinking' })
    expect(within(thinking).queryByRole('button', { name: 'Off' })).not.toBeInTheDocument()
    // Inherited selection with resolved effective model atomically pins on secondary change.
    await user.click(within(thinking).getByRole('button', { name: 'Budget' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ provider: 'codex', model: 'discovered', thinking: 'budget', thinking_budget_tokens: 1024 }))
    const budget = screen.getByRole('spinbutton', { name: 'Thinking budget tokens' })
    expect(budget).toHaveAttribute('min', '1024')
    await user.clear(budget)
    await user.type(budget, '512')
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking_budget_tokens: 1024 }))
    await user.clear(budget)
    await user.type(budget, '2048')
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking_budget_tokens: 2048 }))
    await user.click(within(screen.getByRole('group', { name: 'Mode' })).getByRole('button', { name: 'Pro' }))
    await user.click(within(screen.getByRole('group', { name: 'Summary' })).getByRole('button', { name: 'Detailed' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: 'budget', thinking_budget_tokens: 2048, reasoning_mode: 'pro', reasoning_summary: 'detailed' }))

    // Active non-default secondary settings remain discoverable on the More options button when collapsed.
    await user.click(screen.getByRole('button', { name: /More options/ }))
    expect(screen.getByRole('button', { name: /More options/ })).toHaveTextContent('Budget (2048)')
    expect(screen.getByRole('button', { name: /More options/ })).toHaveTextContent('Pro')
    expect(screen.getByRole('button', { name: /More options/ })).toHaveTextContent('Summary: Detailed')

    await user.click(screen.getByRole('button', { name: /More options/ }))
    await user.click(within(screen.getByRole('group', { name: 'Thinking' })).getByRole('button', { name: 'Adaptive' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: 'adaptive', thinking_budget_tokens: null }))

    // Switching to Other closes the popover and clears unsupported native controls.
    await user.click(screen.getByRole('option', { name: 'Other' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ model: 'other', thinking: null, thinking_budget_tokens: null, reasoning_mode: null, reasoning_summary: null }))
    await user.click(screen.getByRole('button', { name: 'Model: Other, Codex' }))
    expect(screen.queryByRole('button', { name: /More options/ })).not.toBeInTheDocument()
})

it('keeps only the native controls a newly chosen provider offers, so none stay saved but hidden', async () => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: { ...catalog.providers, 'acp:fake': { status: 'available', error: null, label: 'Fake ACP', setup_status: 'ready' } },
        models: [
            { ...catalog.models[0], supported_thinking: ['adaptive', 'budget'], supported_reasoning_modes: ['standard', 'pro'], supported_reasoning_summaries: ['auto', 'detailed'] },
            { ...catalog.models[2], supported_thinking: ['budget'] },
            { provider: 'acp:fake', id: '', display: 'Harness default', is_default: true, supported_reasoning_efforts: [] },
        ],
    })
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor value={{ ...initial, model: 'discovered' }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    await user.click(screen.getByRole('button', { name: /More options/ }))
    await user.click(within(screen.getByRole('group', { name: 'Thinking' })).getByRole('button', { name: 'Budget' }))
    await user.click(within(screen.getByRole('group', { name: 'Mode' })).getByRole('button', { name: 'Pro' }))
    await user.click(within(screen.getByRole('group', { name: 'Summary' })).getByRole('button', { name: 'Detailed' }))
    const native = { ...initial, model: 'discovered', thinking: 'budget', thinking_budget_tokens: 1024, reasoning_mode: 'pro', reasoning_summary: 'detailed' }
    expect(onChange).toHaveBeenLastCalledWith(native)

    // Claude offers budget thinking only: that and its budget stay, the rest is cleared.
    await user.click(screen.getByRole('option', { name: 'Claude' }))
    expect(onChange).toHaveBeenLastCalledWith({ ...native, provider: 'claude-code', model: 'claude-discovered', reasoning_mode: null, reasoning_summary: null })
    await user.click(screen.getByRole('button', { name: /Model: Claude/ }))
    await user.click(screen.getByRole('button', { name: /More options/ }))
    expect(within(screen.getByRole('group', { name: 'Thinking' })).getByRole('button', { name: 'Budget' })).toHaveAttribute('aria-pressed', 'true')

    // The ACP harness offers none of them, so none is left for its turns to reject.
    await user.click(screen.getByRole('option', { name: 'Harness default' }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'acp:fake', model: null, thinking: null, thinking_budget_tokens: null, reasoning_mode: null, reasoning_summary: null })
    await user.click(screen.getByRole('button', { name: /Model: Harness default/ }))
    expect(screen.queryByRole('button', { name: /More options/ })).not.toBeInTheDocument()
})

it('notes a chosen model its provider no longer lists', async () => {
    const view = render(<Editor value={{ ...initial, model: 'retired' }} />)
    const button = await screen.findByRole('button', { name: 'Model: retired, Codex' })
    await waitFor(() => expect(button).toHaveAccessibleDescription('Codex doesn’t list retired right now. Choose a listed model unless it’s a custom one you know works.'))
    view.rerender(<Editor key="listed" value={{ ...initial, model: 'other' }} />)
    expect(await screen.findByRole('button', { name: /Model: Other/ })).not.toHaveAccessibleDescription()
})

it('offers an ACP harness by its label, with a null-model harness default, exact efforts, collapsed locked errors, and next-turn session controls', async () => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: {
            codex: { status: 'available', error: null, label: 'Codex' },
            'acp:fake': { status: 'available', error: null, label: 'Fake ACP', setup_status: 'ready',
                config_options: [
                    { id: 'effort', name: 'Effort', category: 'thought_level', type: 'select', currentValue: 'Lo', options: [{ value: 'Lo', name: 'Low' }, { value: 'Hi/2', name: 'Very high' }] },
                    { id: 'Mode/ID', name: 'Approval', category: 'mode', type: 'select', currentValue: 'Ask', options: [
                        { group: 'manual', name: 'Manual', options: [{ value: 'Ask', name: 'Ask first' }] },
                        { group: 'auto', name: 'Automatic', options: [{ value: 'Auto/Edit', name: 'Auto edit' }] }] },
                    { id: 'verbose', name: 'Verbose', type: 'boolean', currentValue: false },
                ] },
            'acp:locked': { status: 'unavailable', error: 'Sign in first.', label: 'Locked', setup_status: 'unauthenticated' },
        },
        models: [
            { provider: 'codex', id: 'discovered', display: 'Discovered', is_default: true, supported_reasoning_efforts: ['low'] },
            { provider: 'acp:fake', id: '', display: 'Harness default', is_default: true, supported_reasoning_efforts: ['Lo', 'Hi/2'], default_reasoning_effort: 'Lo' },
        ],
    })
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    const harness = screen.getByRole('group', { name: 'Fake ACP', exact: true })
    // Locked unavailable ACP harness is collapsed in footer until expanded.
    expect(screen.queryByRole('group', { name: 'Locked', exact: true })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '1 unavailable provider' }))
    expect(screen.getByRole('group', { name: 'Locked', exact: true })).toHaveTextContent('Authentication required. Model discovery unavailable. Sign in first.')
    expect(screen.queryByRole('group', { name: 'Session controls' })).not.toBeInTheDocument()

    await user.click(within(harness).getByRole('option', { name: 'Harness default' }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'acp:fake', model: null })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /^Reasoning effort:/ }))
    await user.click(screen.getByRole('button', { name: 'Very high' }))
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'acp:fake', model: null, reasoning_effort: 'Hi/2' })

    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    await user.click(screen.getByRole('button', { name: /More options/ }))
    const controls = screen.getByRole('group', { name: 'Session controls' })
    expect(controls).toHaveTextContent('changes apply from the next turn')
    expect(within(controls).getAllByRole('combobox').map(select => select.getAttribute('aria-label'))).toEqual(['Approval'])
    await user.selectOptions(within(controls).getByRole('combobox', { name: 'Approval' }), 'Auto/Edit')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'acp:fake', model: null, reasoning_effort: 'Hi/2', session_config: { 'Mode/ID': 'Auto/Edit' } })
    await user.selectOptions(within(controls).getByRole('combobox', { name: 'Approval' }), 'Harness default')
    expect(onChange.mock.lastCall?.[0].session_config).toEqual({})

    // Saved choices the harness no longer offers auto-expand More options so they stay visible until cleared.
    cleanup()
    render(<Editor value={{ ...initial, provider: 'acp:fake', session_config: { 'Mode/ID': 'gone', retired: 'on' } }} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /^Model:/ }))
    const stale = screen.getByRole('group', { name: 'Session controls' })
    expect(within(stale).getByRole('combobox', { name: 'Approval' })).toHaveDisplayValue('gone (not offered now)')
    await user.selectOptions(within(stale).getByRole('combobox', { name: 'retired' }), 'Harness default')
    expect(onChange.mock.lastCall?.[0].session_config).toEqual({ 'Mode/ID': 'gone' })

    // Another provider starts without the harness's controls.
    await user.click(screen.getByRole('option', { name: 'Discovered' }))
    expect(onChange.mock.lastCall?.[0]).toMatchObject({ provider: 'codex', model: 'discovered' })
    expect(onChange.mock.lastCall?.[0].session_config).toBeUndefined()
    await user.click(screen.getByRole('button', { name: /^Model:/ }))
    expect(screen.queryByRole('group', { name: 'Session controls' })).not.toBeInTheDocument()
})

it('refetches quietly on open, so values a harness acknowledged since can be chosen', async () => {
    const select = (id: string, category: string | undefined, values: string[]) => ({ id, name: id, category, type: 'select', currentValue: values[0], options: values.map(value => ({ value, name: value })) })
    const offering = (efforts: string[], approvals: string[]): ProjectChatModelsResponse => ({
        providers: { 'acp:fake': { status: 'available', error: null, label: 'Fake ACP', setup_status: 'ready',
            config_options: [select('effort', 'thought_level', efforts), select('Approval', undefined, approvals)] } },
        models: [{ provider: 'acp:fake', id: 'deep', display: 'Deep', is_default: true, supported_reasoning_efforts: efforts }],
    })
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(offering(['Low'], ['ask']))
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor value={{ ...initial, provider: 'acp:fake', model: 'deep' }} onChange={onChange} />)
    const effortTrigger = await screen.findByRole('button', { name: /^Reasoning effort:/ })
    await user.click(effortTrigger)
    expect(within(screen.getByRole('group', { name: 'Reasoning effort' })).queryByRole('button', { name: 'High' })).not.toBeInTheDocument()
    await user.keyboard('{Escape}')

    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(offering(['Low', 'High'], ['ask', 'auto']))
    await user.click(effortTrigger)
    expect(screen.queryByText('Loading models…')).not.toBeInTheDocument()
    await user.click(await within(screen.getByRole('group', { name: 'Reasoning effort' })).findByRole('button', { name: 'High' }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ provider: 'acp:fake', model: 'deep', reasoning_effort: 'High' }))

    await user.click(screen.getByRole('button', { name: /Model: Deep/ }))
    await user.click(screen.getByRole('button', { name: /More options/ }))
    await user.selectOptions(within(screen.getByRole('group', { name: 'Session controls' })).getByRole('combobox', { name: 'Approval' }), 'auto')
    expect(onChange.mock.lastCall?.[0].session_config).toEqual({ Approval: 'auto' })
})

it('preserves inherited Thinking, Mode, Summary, and ACP session controls when pinning the same effective model on Effort change', async () => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({
        providers: {
            ...catalog.providers,
            'acp:fake': {
                status: 'available',
                error: null,
                label: 'Fake ACP',
                setup_status: 'ready',
                config_options: [
                    { id: 'effort', name: 'Effort', category: 'thought_level', type: 'select', currentValue: 'Lo', options: [{ value: 'Lo', name: 'Low' }, { value: 'Hi', name: 'High' }] },
                    { id: 'Approval', name: 'Approval', type: 'select', currentValue: 'ask', options: [{ value: 'ask', name: 'Ask' }, { value: 'auto', name: 'Auto' }] },
                ],
            },
        },
        models: [
            { ...catalog.models[0], supported_thinking: ['adaptive', 'budget'], supported_reasoning_modes: ['standard', 'pro'], supported_reasoning_summaries: ['auto', 'detailed'] },
            { provider: 'acp:fake', id: 'deep', display: 'Deep', is_default: true, supported_reasoning_efforts: ['Lo', 'Hi'], default_reasoning_effort: 'Lo' },
        ],
    })
    const user = userEvent.setup(), onChange = vi.fn()
    const inheritedNative: ModelSettings = {
        provider: 'codex',
        llm_profile: null,
        model: 'discovered',
        reasoning_effort: 'low',
        thinking: 'budget',
        thinking_budget_tokens: 2048,
        reasoning_mode: 'pro',
        reasoning_summary: 'detailed',
    }
    const view = render(<Editor value={cleared} inherited={inheritedNative} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /^Reasoning effort:/ }))
    await user.click(screen.getByRole('button', { name: 'Ultra', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({
        ...inheritedNative,
        reasoning_effort: 'ultra',
    })

    // Also preserve ACP session_config when pinning an inherited ACP model for effort.
    const inheritedAcp: ModelSettings = {
        provider: 'acp:fake',
        llm_profile: null,
        model: 'deep',
        reasoning_effort: 'Lo',
        session_config: { Approval: 'auto' },
    }
    view.rerender(<Editor key="acp-inherit" value={cleared} inherited={inheritedAcp} onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /^Reasoning effort:/ }))
    await user.click(screen.getByRole('button', { name: 'High', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith({
        ...inheritedAcp,
        reasoning_effort: 'Hi',
    })
})

it('does not pre-highlight custom model on mistyped search + Enter, requiring explicit selection', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    const search = screen.getByRole('combobox', { name: 'Search models' })
    await user.type(search, 'typo-model')
    expect(search).not.toHaveAttribute('aria-activedescendant')
    await user.keyboard('{Enter}')
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.getByRole('listbox')).toBeVisible()

    // Deliberately pressing ArrowDown highlights the custom option, then Enter commits it.
    await user.keyboard('{ArrowDown}{Enter}')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...initial, model: 'typo-model' })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
})

it('omits unconfigured profiles from Pinned and Recent quick sections', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([
        { id: 'ready', label: 'Ready', provider: 'openai', configured: true, models: ['ready-one'] },
        { id: 'missing-creds', label: 'Missing Creds', provider: 'openai', configured: false, models: ['secret-one'] },
    ])
    localStorage.setItem('spark.model_chooser.pinned', JSON.stringify([
        { provider: null, llm_profile: 'missing-creds', model: 'secret-one' },
        { provider: null, llm_profile: 'ready', model: 'ready-one' },
    ]))
    localStorage.setItem('spark.model_chooser.recents', JSON.stringify([
        { provider: null, llm_profile: 'missing-creds', model: 'secret-one' },
    ]))
    const user = userEvent.setup()
    render(<Editor />)
    await user.click(await screen.findByRole('button', { name: /Model: Discovered/ }))
    const pinnedGroup = screen.getByRole('group', { name: 'Pinned' })
    expect(within(pinnedGroup).getByRole('option', { name: /ready-one/ })).toBeInTheDocument()
    expect(within(pinnedGroup).queryByRole('option', { name: /secret-one/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Recent' })).not.toBeInTheDocument()
})

it('keeps saved ACP session controls and saved native secondary settings inspectable and clearable when discovery metadata is missing', async () => {
    const user = userEvent.setup(), onChange = vi.fn()
    // Harness 'acp:removed' is completely absent from discovery.providers.
    const view = render(
        <Editor
            value={{ provider: 'acp:removed', llm_profile: null, model: 'gone-model', reasoning_effort: null, session_config: { Approval: 'auto' } }}
            onChange={onChange}
        />,
    )
    await user.click(await screen.findByRole('button', { name: /Model: gone-model/ }))
    const controls = screen.getByRole('group', { name: 'Session controls' })
    expect(within(controls).getByRole('combobox', { name: 'Approval' })).toHaveDisplayValue('auto (not offered now)')
    await user.selectOptions(within(controls).getByRole('combobox', { name: 'Approval' }), 'Harness default')
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ provider: 'acp:removed', session_config: {} }))
    await user.keyboard('{Escape}')

    // Saved native secondary settings on an unlisted model with missing capability metadata stay inspectable & clearable.
    view.rerender(
        <Editor
            key="missing-meta"
            value={{
                provider: 'codex',
                llm_profile: null,
                model: 'unlisted-model',
                reasoning_effort: null,
                thinking: 'budget',
                thinking_budget_tokens: 2048,
                reasoning_mode: 'pro',
                reasoning_summary: 'detailed',
            }}
            onChange={onChange}
        />,
    )
    await user.click(screen.getByRole('button', { name: /Model: unlisted-model/ }))
    expect(screen.getByRole('group', { name: 'Thinking' })).toBeVisible()
    expect(screen.getByRole('spinbutton', { name: 'Thinking budget tokens' })).toHaveValue(2048)
    await user.click(within(screen.getByRole('group', { name: 'Thinking' })).getByRole('button', { name: 'Default', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ thinking: null, thinking_budget_tokens: null }))
    await user.click(within(screen.getByRole('group', { name: 'Mode' })).getByRole('button', { name: 'Standard', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ reasoning_mode: null }))
    await user.click(within(screen.getByRole('group', { name: 'Summary' })).getByRole('button', { name: 'Default', exact: true }))
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ reasoning_summary: null }))
})
