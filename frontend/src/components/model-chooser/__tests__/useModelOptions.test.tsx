import { act, renderHook } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { resetModelOptionsCache, useModelOptions } from '../useModelOptions'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import { deferred } from '@/test/environments'

vi.mock('@/lib/api/projectsApi', () => ({ fetchProjectChatModelsValidated: vi.fn() }))

const models = (label: string): ProjectChatModelsResponse => ({ models: [], providers: { codex: { status: 'available', error: null, label } } })

afterEach(() => {
    resetModelOptionsCache()
    vi.resetAllMocks()
})

it('keeps the entry that replaced a reset one when the retired entry cleans up and answers late', async () => {
    const retired = deferred<ProjectChatModelsResponse>()
    const replacement = deferred<ProjectChatModelsResponse>()
    vi.mocked(fetchProjectChatModelsValidated).mockReturnValueOnce(retired.promise).mockReturnValueOnce(replacement.promise)

    const old = renderHook(() => useModelOptions('/tmp/project'))
    // An environment switch resets the cache and remounts its consumers for the same project.
    resetModelOptionsCache()
    old.unmount()
    const current = renderHook(() => useModelOptions('/tmp/project'))
    // The old consumer's cleanup runs in a microtask.
    await act(async () => { await Promise.resolve() })

    await act(async () => { replacement.resolve(models('Replacement')) })
    await act(async () => { retired.resolve(models('Retired')) })
    expect(fetchProjectChatModelsValidated).toHaveBeenCalledTimes(2)
    expect(current.result.current?.payload).toEqual(models('Replacement'))
})
