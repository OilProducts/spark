import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { fetchModelSettings } from '@/lib/api/settingsApi'
import { useModelSettingsEditor } from '../hooks/useModelSettingsEditor'

vi.mock('@/lib/api/settingsApi', () => ({ fetchModelSettings: vi.fn(), saveModelSettings: vi.fn() }))
vi.mock('../hooks/useSettingsNavigationProtection', () => ({ useSettingsNavigationProtection: vi.fn() }))
afterEach(() => vi.restoreAllMocks())

it('refetches inherited effective values when the workspace changes without changing the project revision, retaining dirty drafts', async () => {
    const group = (model: string) => ({ provider: 'codex', llm_profile: null, model, reasoning_effort: null })
    const view = (model: string) => ({ scope: 'project' as const, source: 'workspace' as const, revision: 'project-1', stored: null, effective: group(model) })
    vi.mocked(fetchModelSettings).mockResolvedValue(view('workspace-one'))
    const { result } = renderHook(() => useModelSettingsEditor('/projects/scoped'))
    await waitFor(() => expect(result.current.saved?.effective.model).toBe('workspace-one'))
    vi.mocked(fetchModelSettings).mockResolvedValue(view('workspace-two'))
    act(() => window.dispatchEvent(new Event('focus')))
    await waitFor(() => expect(result.current.saved?.effective.model).toBe('workspace-two'))
    expect(result.current.draft).toBeNull()
    act(() => result.current.setDraft(group('my-draft')))
    vi.mocked(fetchModelSettings).mockResolvedValue(view('workspace-three'))
    act(() => window.dispatchEvent(new Event('focus')))
    await waitFor(() => expect(result.current.message).toContain('Your draft is retained'))
    expect(result.current.draft?.model).toBe('my-draft')
    await act(() => result.current.discard())
    expect(result.current.draft).toBeNull()
    expect(result.current.saved?.effective.model).toBe('workspace-three')
})
