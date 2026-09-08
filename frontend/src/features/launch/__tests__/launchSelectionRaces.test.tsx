import { act, renderHook } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchPipelineStartValidated, fetchPipelineContinueValidated, type PipelineStartResponse } from '@/lib/attractorClient'
import { useStartPipeline } from '../hooks/useStartPipeline'

vi.mock('@/lib/workspaceClient', () => ({ fetchProjectMetadataValidated: vi.fn().mockResolvedValue({ branch: 'main' }) }))
vi.mock('@/lib/attractorClient', async (original) => ({
    ...await original<typeof import('@/lib/attractorClient')>(),
    fetchPipelineStartValidated: vi.fn(), fetchPipelineContinueValidated: vi.fn(),
}))
const form = { projectPath: '/a', workingDirectory: '/a', flowSource: 'flow', model: null }
let complete: (response: PipelineStartResponse) => void
beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    const state = useStore.getState()
    state.registerProject('/a')
    state.registerProject('/b')
    state.setRunsSelectedRunIdForScope('project:/b', 'b')
    const delayed = () => new Promise<PipelineStartResponse>((resolve) => { complete = resolve })
    vi.mocked(fetchPipelineStartValidated).mockImplementation(delayed)
    vi.mocked(fetchPipelineContinueValidated).mockImplementation(delayed)
})

it.each(['launch', 'continuation'])('keeps a delayed %s in its captured project scope', async (kind) => {
    const { result } = renderHook(() => useStartPipeline(), { wrapper: DialogProvider })
    let request!: Promise<unknown>
    await act(async () => {
        request = kind === 'launch'
            ? result.current.startFromFlowContent(form, 'flow')
            : result.current.continueFromRun('source', form, { startNodeId: 'review', flowSourceMode: 'snapshot' })
    })
    act(() => useStore.getState().setActiveProjectPath('/b'))
    await act(async () => { complete({ status: 'started', pipeline_id: 'new-a' }); await request })
    expect(useStore.getState().runsListSession.selectedRunIdByScopeKey).toMatchObject({ 'project:/a': 'new-a', 'project:/b': 'b' })
    expect(useStore.getState().runDetailSessionsByRunId['new-a'].record).toBeNull()
})

it('does not recreate selections or sessions for a removed launch project', async () => {
    const { result } = renderHook(() => useStartPipeline(), { wrapper: DialogProvider })
    let request!: Promise<unknown>
    await act(async () => { request = result.current.startFromFlowContent(form, 'flow') })
    act(() => useStore.getState().removeProject('/a', '/b'))
    await act(async () => { complete({ status: 'queued', pipeline_id: 'removed-a' }); await request })
    expect(useStore.getState().runsListSession.selectedRunIdByScopeKey['project:/a']).toBeUndefined()
    expect(useStore.getState().runDetailSessionsByRunId['removed-a']).toBeUndefined()
})
