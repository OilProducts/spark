import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { fetchProjectBrowseValidated, fetchProjectMetadataValidated, registerProjectValidated } from '@/lib/workspaceClient'
import { useAddProject } from '../useAddProject'

vi.mock('@/lib/workspaceClient', async (importOriginal) => ({
    ...await importOriginal<typeof import('@/lib/workspaceClient')>(),
    fetchProjectBrowseValidated: vi.fn(),
    fetchProjectMetadataValidated: vi.fn(),
    registerProjectValidated: vi.fn(),
}))

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

it('sends a UNC folder to the Git preflight and registration with its share prefix intact', async () => {
    const uncPath = '\\\\fileserver\\share\\team\\repo'
    vi.mocked(fetchProjectBrowseValidated).mockResolvedValue({ current_path: uncPath, parent_path: '\\\\fileserver\\share\\team', roots: [], entries: [] })
    vi.mocked(fetchProjectMetadataValidated).mockResolvedValue({ branch: 'main', commit: 'abc123' })
    vi.mocked(registerProjectValidated).mockResolvedValue({
        project_id: 'repo', project_path: uncPath, display_name: 'repo', created_at: '2026-11-01T00:00:00Z',
        last_opened_at: '2026-11-01T00:00:00Z', is_favorite: false, is_default: false, folder_exists: true,
    })
    const { result } = renderHook(() => useAddProject())

    await act(() => result.current.onOpenProjectDirectoryChooser())
    await act(() => result.current.onSelectProjectBrowserDirectory())

    expect(fetchProjectMetadataValidated).toHaveBeenCalledWith('//fileserver/share/team/repo')
    expect(registerProjectValidated).toHaveBeenCalledWith('//fileserver/share/team/repo')
    expect(useStore.getState().projectRegistry['//fileserver/share/team/repo']).toBeDefined()
    expect(useStore.getState().projectRegistrationError).toBeNull()
})
