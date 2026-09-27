import { StrictMode, useState } from 'react'
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ModelChooser } from '../ModelChooser'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions'

vi.mock('@/lib/api/projectsApi', () => ({ fetchProjectChatModelsValidated: vi.fn() }))
vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: vi.fn() }))
const initial: ModelSettings = { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null }
const catalog: ProjectChatModelsResponse = {
    providers: { codex: { status: 'available', error: null } },
    models: [
        { provider: 'codex', id: 'discovered', display: 'Discovered', is_default: true, supported_reasoning_efforts: ['low', 'ultra'] },
        { provider: 'codex', id: 'other', display: 'Other', is_default: false, supported_reasoning_efforts: ['medium'] },
        { provider: 'claude-code', id: 'claude-discovered', display: 'Claude', is_default: true, supported_reasoning_efforts: [] },
    ],
}
function Editor({ value = initial, projectPath = '/project', onChange = vi.fn(), layout = 'fields', disabled = false }: {
    value?: ModelSettings; projectPath?: string | null; onChange?: (value: ModelSettings) => void; layout?: 'fields' | 'compact'; disabled?: boolean
}) {
    const [draft, setDraft] = useState(value)
    return <ModelChooser value={draft} onChange={(next) => { setDraft(next); onChange(next) }}
        projectPath={projectPath} inheritLabel="Graph default" layout={layout} disabled={disabled} />
}
beforeEach(() => {
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', label: 'Team', provider: 'openai', configured: true, models: ['team-one', 'team-two'] }])
})
afterEach(async () => { cleanup(); await Promise.resolve(); vi.resetAllMocks() })

it('emits full settings for provider, profile, model, effort and inheritance selections', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Editor onChange={onChange} />)
    const provider = screen.getByLabelText('Provider or profile')
    for (const option of LLM_PROVIDER_OPTIONS) expect(within(provider).getByRole('option', { name: option })).toBeInTheDocument()
    expect(screen.getAllByRole('option', { name: 'Graph default' })).toHaveLength(3)
    await user.selectOptions(provider, 'openai')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'openai' })
    await user.selectOptions(screen.getByLabelText('Model', { exact: true }), 'model:gpt-5.5')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'openai', model: 'gpt-5.5' })
    await user.selectOptions(screen.getByLabelText('Reasoning effort'), 'high')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: 'openai', model: 'gpt-5.5', reasoning_effort: 'high' })
    await user.selectOptions(provider, 'team')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-one' })
    await user.selectOptions(screen.getByLabelText('Model', { exact: true }), 'model:team-two')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: null, llm_profile: 'team', model: 'team-two' })
    await user.selectOptions(provider, '')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, provider: null })
})

it('prioritizes discovered models and changes efforts with the model, falling back when effort metadata is absent', async () => {
    const user = userEvent.setup()
    render(<Editor value={{ ...initial, model: 'discovered' }} />)
    await screen.findByRole('option', { name: 'discovered' })
    expect(screen.queryByRole('option', { name: 'gpt-5.5' })).not.toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'Ultra' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'High' })).not.toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Model', { exact: true }), 'model:other')
    expect(screen.getByRole('option', { name: 'Medium' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'Ultra' })).not.toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Provider or profile'), 'claude-code')
    expect(screen.getByRole('option', { name: 'claude-discovered' })).toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Model', { exact: true }), 'model:claude-discovered')
    expect(within(screen.getByLabelText('Reasoning effort')).getAllByRole('option')).toHaveLength(7)
})

it('prioritizes profile models over discovery and suggestions, retaining profile defaults', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'codex', provider: 'openai', configured: true, models: ['profile-only'], default_model: 'profile-only' }])
    render(<Editor value={{ ...initial, provider: null, llm_profile: 'codex' }} />)
    await waitFor(() => expect(fetchProjectChatModelsValidated).toHaveBeenCalledOnce())
    expect(screen.getByRole('option', { name: 'profile-only' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'discovered' })).not.toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'gpt-5.5' })).not.toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'Max' })).toBeInTheDocument()
})

it('uses provider-scoped effort metadata for profile models and provider defaults', async () => {
    vi.mocked(useLlmProfiles).mockReturnValue([{ id: 'team', provider: 'codex', configured: true,
        models: ['discovered'], default_model: 'discovered' }])
    const view = render(<Editor />)
    await screen.findByRole('option', { name: 'discovered' })
    expect(screen.queryByRole('option', { name: 'High' })).not.toBeInTheDocument()
    view.rerender(<Editor key="profile" value={{ ...initial, provider: null, llm_profile: 'team' }} />)
    expect(screen.getByRole('option', { name: 'Ultra' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'High' })).not.toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'other' })).not.toBeInTheDocument()
})

it('supports custom entry, labels unlisted saved values and preserves unknown efforts in compact layout', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Editor layout="compact" value={{ ...initial, model: 'unlisted', reasoning_effort: 'future' }} onChange={onChange} />)
    expect(screen.getByRole('option', { name: 'unlisted (custom)' })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'Future (custom)' })).toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Model', { exact: true }), 'custom')
    await user.clear(screen.getByLabelText('Custom model'))
    await user.type(screen.getByLabelText('Custom model'), 'my-model')
    expect(onChange).toHaveBeenLastCalledWith({ ...initial, model: 'my-model', reasoning_effort: 'future' })
})

it('disables every control when disabled', async () => {
    render(<Editor disabled value={{ ...initial, model: 'custom-model' }} />)
    await screen.findByRole('option', { name: 'discovered' })
    for (const control of [...screen.getAllByRole('combobox'), screen.getByRole('textbox')]) expect(control).toBeDisabled()
})

it('shares loading and failed discovery with suggestion fallbacks across StrictMode choosers and refreshes once', async () => {
    let reject!: (error: Error) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValue(new Promise((_, fail) => { reject = fail }))
    const view = render(<StrictMode><Editor /><Editor layout="compact" /></StrictMode>)
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledExactlyOnceWith('/project')
    expect(screen.getAllByText('Loading models…')).toHaveLength(2)
    expect(screen.getAllByRole('option', { name: 'gpt-5.5' })).toHaveLength(2)
    await act(async () => reject(new Error('offline')))
    expect(screen.getAllByText('Model discovery unavailable. Using suggestions.')).toHaveLength(2)
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue(catalog)
    act(() => window.dispatchEvent(new Event('spark:codex-connected')))
    await waitFor(() => expect(screen.getAllByRole('option', { name: 'discovered' })).toHaveLength(2))
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(2)
    view.rerender(<StrictMode><Editor /><Editor layout="compact" /><Editor /></StrictMode>)
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(2)
})

it('keeps projects isolated and ignores pre-refresh responses', async () => {
    let resolve!: (value: ProjectChatModelsResponse) => void
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const view = render(<Editor projectPath="/one" />)
    act(() => window.dispatchEvent(new Event('spark:codex-connected')))
    await screen.findByRole('option', { name: 'discovered' })
    await act(async () => resolve({ ...catalog, models: [] }))
    expect(screen.getByRole('option', { name: 'discovered' })).toBeInTheDocument()
    vi.mocked(fetchProjectChatModelsValidated).mockResolvedValue({ models: [], providers: { codex: { status: 'unavailable', error: 'offline' } } })
    view.rerender(<Editor projectPath="/two" />)
    await screen.findByText('Model discovery unavailable. Using suggestions.')
    expect(screen.queryByRole('option', { name: 'discovered' })).not.toBeInTheDocument()
    expect(fetchProjectChatModelsValidated).toHaveBeenLastCalledWith('/two')
})
