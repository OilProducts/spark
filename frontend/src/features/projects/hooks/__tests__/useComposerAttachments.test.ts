import { act, renderHook } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { useComposerAttachments } from '@/features/projects/hooks/useComposerAttachments'
import { buildProjectConversationId } from '@/features/projects/model/projectsHomeState'
import { deleteConversationAttachmentValidated, uploadConversationAttachmentValidated } from '@/lib/workspaceClient'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { deferred, switchEnvironment } from '@/test/environments'

vi.mock('@/lib/workspaceClient', async (original) => ({
    ...await original<object>(),
    uploadConversationAttachmentValidated: vi.fn(),
    deleteConversationAttachmentValidated: vi.fn(async () => undefined),
}))

afterEach(() => {
    vi.unstubAllGlobals()
})

it('attaches a file in a new chat from a non-secure origin, where crypto.randomUUID is missing', () => {
    vi.stubGlobal('crypto', { getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto) })
    vi.mocked(uploadConversationAttachmentValidated).mockReturnValue(new Promise(() => {}))
    const projectPath = '/tmp/project'
    const file = new File(['hello'], 'notes.txt', { type: 'text/plain' })
    // A first attachment creates the chat, as the controller's ensureConversationId does.
    const conversationId = buildProjectConversationId(projectPath)
    const { result } = renderHook(() => useComposerAttachments({
        activeConversationId: conversationId,
        activeProjectPath: projectPath,
        ensureConversationId: () => conversationId,
        setPanelError: vi.fn(),
    }))

    act(() => result.current.addFiles([file]))

    expect(conversationId).toMatch(/^conversation-tmp-project-[a-z0-9]+$/)
    expect(useStore.getState().homeConversationSessionsById[conversationId]?.attachments).toMatchObject([
        { localId: expect.stringMatching(/^[0-9a-f]{32}$/), file, status: 'uploading' },
    ])
    expect(uploadConversationAttachmentValidated).toHaveBeenCalledWith(conversationId, projectPath, file, null)
})

it('leaves the environment switched to as it is when an upload finishes there, removing the copy it left', async () => {
    type Uploaded = Awaited<ReturnType<typeof uploadConversationAttachmentValidated>>
    const uploaded = deferred<Uploaded>()
    vi.mocked(uploadConversationAttachmentValidated).mockReturnValue(uploaded.promise)
    const projectPath = '/tmp/project'
    const conversationId = 'conversation-1'
    await switchEnvironment('env-r')
    const { result } = renderHook(() => useComposerAttachments({
        activeConversationId: conversationId,
        activeProjectPath: projectPath,
        ensureConversationId: () => conversationId,
        setPanelError: vi.fn(),
    }))
    act(() => result.current.addFiles([new File(['hello'], 'notes.txt', { type: 'text/plain' })]))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)

    await act(async () => uploaded.resolve({ id: 'attachment-1' } as Uploaded))

    expect(useStore.getState().homeConversationSessionsById).toEqual({})
    expect(deleteConversationAttachmentValidated).toHaveBeenCalledWith(conversationId, 'attachment-1', projectPath, 'env-r')
})
