import { HomeSessionController } from '@/app/AppSessionControllers'
import { ProjectsPanel } from '@/features/projects/ProjectsPanel'
import { useStore } from '@/store'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const PROJECT = '/tmp/attach-project'

const json = (payload: unknown, status = 200) => new Response(JSON.stringify(payload), {
  status,
  headers: { 'Content-Type': 'application/json' },
})

const url = (input: RequestInfo | URL) => (typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url)

const snapshot = (conversationId: string, turns: unknown[] = []) => ({
  schema_version: 5,
  revision: turns.length,
  conversation_id: conversationId,
  project_path: PROJECT,
  title: conversationId === 'chat-a' ? 'Chat A' : 'Chat B',
  created_at: '2026-10-01T10:00:00Z',
  updated_at: '2026-10-01T10:00:00Z',
  turns,
  segments: [],
  event_log: [],
})

type Upload = { conversationId: string; filename: string; size: number }

/** A fetch double for the chat endpoints; uploads answer with sequential ids. */
function stubServer({ failUploads = 0, failTurns = 0 } = {}) {
  const uploads: Upload[] = []
  const deletes: string[] = []
  const turns: Array<{ conversationId: string; body: Record<string, unknown> }> = []
  let uploadFailures = failUploads
  let turnFailures = failTurns
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const requestUrl = url(input)
    const parsed = new URL(requestUrl, 'http://spark.test')
    const conversationId = decodeURIComponent(parsed.pathname.match(/\/conversations\/([^/]+)/)?.[1] ?? '')
    if (requestUrl.includes('/projects/chat-models')) {
      return json({
        providers: { codex: { status: 'available', error: null } },
        models: [{ provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: [] }],
      })
    }
    if (requestUrl.includes('/projects/conversations')) {
      return json([
        { conversation_id: 'chat-a', project_path: PROJECT, title: 'Chat A', created_at: '2026-10-01T10:00:00Z', updated_at: '2026-10-01T10:00:00Z', revision: 0 },
        { conversation_id: 'chat-b', project_path: PROJECT, title: 'Chat B', created_at: '2026-10-01T09:00:00Z', updated_at: '2026-10-01T09:00:00Z', revision: 0 },
      ])
    }
    if (parsed.pathname.endsWith('/attachments') && init?.method === 'POST') {
      if (uploadFailures > 0) {
        uploadFailures -= 1
        return json({ detail: 'Disk is full.' }, 503)
      }
      const body = init.body as File
      uploads.push({ conversationId, filename: parsed.searchParams.get('filename') ?? '', size: body.size })
      return json({
        id: `att-${uploads.length}`,
        filename: parsed.searchParams.get('filename'),
        media_type: body.type || 'application/octet-stream',
        size_bytes: body.size,
      })
    }
    if (parsed.pathname.includes('/attachments/') && init?.method === 'DELETE') {
      deletes.push(parsed.pathname.split('/').at(-1)!)
      return json({ status: 'deleted' })
    }
    if (parsed.pathname.endsWith('/turns')) {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>
      turns.push({ conversationId, body })
      if (turnFailures > 0) {
        turnFailures -= 1
        return json({ detail: 'backend unavailable' }, 503)
      }
      const ids = (body.attachment_ids as string[] | undefined) ?? []
      return json(snapshot(conversationId, [{
        id: 'turn-user-1',
        role: 'user',
        content: body.message,
        timestamp: '2026-10-01T10:01:00Z',
        status: 'complete',
        kind: 'message',
        attachments: ids.map((id) => {
          const upload = uploads[Number(id.slice(4)) - 1]!
          return {
            id,
            filename: upload.filename,
            media_type: upload.filename.endsWith('.png') ? 'image/png' : 'text/plain',
            size_bytes: upload.size,
          }
        }),
      }, {
        id: 'turn-assistant-1',
        role: 'assistant',
        content: 'Seen.',
        timestamp: '2026-10-01T10:01:01Z',
        status: 'complete',
        kind: 'message',
        parent_turn_id: 'turn-user-1',
      }]))
    }
    if (conversationId && !init?.method) {
      return json(snapshot(conversationId))
    }
    return json({})
  })
  vi.stubGlobal('fetch', fetchMock)
  return { uploads, deletes, turns }
}

const png = (name = 'screenshot.png') => new File(['png-bytes'], name, { type: 'image/png' })
const text = (name = 'notes.txt') => new File(['some notes'], name, { type: 'text/plain' })

const renderChat = (conversationId = 'chat-a') => {
  act(() => {
    useStore.getState().registerProject(PROJECT)
    useStore.getState().setActiveProjectPath(PROJECT)
    useStore.getState().updateProjectSessionState(PROJECT, { conversationId })
  })
  return render(<><HomeSessionController /><ProjectsPanel /></>)
}

const input = () => screen.getByTestId('project-ai-conversation-input')
const sendButton = () => screen.getByTestId('project-ai-conversation-send-button')
const pendingAttachments = () => screen.queryAllByTestId('composer-attachment')

async function waitForUploads(count: number) {
  await waitFor(() => {
    const entries = pendingAttachments()
    expect(entries).toHaveLength(count)
    entries.forEach((entry) => expect(entry).toHaveAttribute('data-status', 'uploaded'))
  })
}

describe('composer attachments', () => {
  beforeEach(() => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, writable: true, value: 1280 })
    window.dispatchEvent(new Event('resize'))
    useStore.setState({
      ...useStore.getState(),
      viewMode: 'projects',
      activeProjectPath: null,
      projectRegistry: {},
      projectSessionsByPath: {},
      homeConversationCache: { conversationsById: {}, summariesByProjectPath: {} },
      homeThreadSummariesStatusByProjectPath: {},
      homeThreadSummariesErrorByProjectPath: {},
      homeProjectSessionsByPath: {},
      homeConversationSessionsById: {},
    })
    URL.createObjectURL = vi.fn(() => 'blob:preview')
    URL.revokeObjectURL = vi.fn()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('keeps ordinary text paste in the message box', async () => {
    const user = userEvent.setup()
    stubServer()
    renderChat()

    await user.click(input())
    await user.paste('Pasted words')

    expect(input()).toHaveValue('Pasted words')
    expect(pendingAttachments()).toHaveLength(0)
  })

  it('attaches pasted, dropped and selected files and sends them without text', async () => {
    const user = userEvent.setup()
    const server = stubServer()
    renderChat()

    fireEvent.paste(input(), { clipboardData: { files: [png()], types: ['Files'] } })
    fireEvent.drop(screen.getByTestId('project-ai-conversation-composer'), {
      dataTransfer: { files: [text()], types: ['Files'] },
    })
    await user.upload(screen.getByTestId('project-ai-conversation-file-input'), text('report.txt'))

    await waitForUploads(3)
    expect(screen.getByRole('img', { name: 'screenshot.png' })).toHaveAttribute('src', 'blob:preview')
    expect(screen.getByTestId('composer-attachment-limits')).toHaveTextContent('3 of 10 files')
    expect(server.uploads.map((upload) => upload.filename)).toEqual(['screenshot.png', 'notes.txt', 'report.txt'])
    expect(input()).toHaveValue('')
    expect(sendButton()).toBeEnabled()

    await user.click(sendButton())

    await waitFor(() => expect(pendingAttachments()).toHaveLength(0))
    expect(server.turns).toEqual([{
      conversationId: 'chat-a',
      body: { project_path: PROJECT, message: '', attachment_ids: ['att-1', 'att-2', 'att-3'], chat_mode: null },
    }])
    const history = await screen.findByTestId('message-attachments')
    expect(within(history).getByRole('img', { name: 'screenshot.png' })).toHaveAttribute(
      'src',
      `/workspace/api/conversations/chat-a/attachments/att-1?project_path=${encodeURIComponent(PROJECT)}`,
    )
    expect(within(history).getByRole('link', { name: /notes\.txt/ })).toHaveAttribute('download', 'notes.txt')
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:preview')
  })

  it('blocks Send on a failed upload until retried, and removes attachments', async () => {
    const user = userEvent.setup()
    const server = stubServer({ failUploads: 1 })
    renderChat()

    await user.type(input(), 'Look at this')
    fireEvent.paste(input(), { clipboardData: { files: [png()], types: ['Files'] } })

    await waitFor(() => expect(pendingAttachments()[0]).toHaveAttribute('data-status', 'failed'))
    expect(pendingAttachments()[0]).toHaveTextContent('Disk is full.')
    expect(sendButton()).toBeDisabled()

    await user.click(screen.getByRole('button', { name: 'Retry uploading screenshot.png' }))
    await waitForUploads(1)
    expect(sendButton()).toBeEnabled()

    await user.click(screen.getByRole('button', { name: 'Remove screenshot.png' }))
    expect(pendingAttachments()).toHaveLength(0)
    await waitFor(() => expect(server.deletes).toEqual(['att-1']))
    expect(input()).toHaveValue('Look at this')
  })

  it('keeps text and attachments when the send fails', async () => {
    const user = userEvent.setup()
    const server = stubServer({ failTurns: 1 })
    renderChat()

    await user.type(input(), 'Retry me')
    fireEvent.paste(input(), { clipboardData: { files: [png()], types: ['Files'] } })
    await waitForUploads(1)
    await user.click(sendButton())

    expect(await screen.findByText('backend unavailable')).toBeVisible()
    expect(input()).toHaveValue('Retry me')
    expect(pendingAttachments()).toHaveLength(1)

    await user.click(sendButton())
    await waitFor(() => expect(pendingAttachments()).toHaveLength(0))
    expect(server.turns.map((turn) => turn.body.attachment_ids)).toEqual([['att-1'], ['att-1']])
  })

  it('refuses files over the limits before uploading', async () => {
    const user = userEvent.setup()
    const server = stubServer()
    renderChat()
    const huge = text('huge.log')
    Object.defineProperty(huge, 'size', { value: 20 * 1024 * 1024 + 1 })

    await user.upload(screen.getByTestId('project-ai-conversation-file-input'), [huge, ...Array.from({ length: 11 }, (_, index) => text(`file-${index}.txt`))])

    expect(screen.getByTestId('project-panel-error')).toHaveTextContent('huge.log is larger than the 20 MB per-file limit.')
    expect(screen.getByTestId('project-panel-error')).toHaveTextContent('A message can carry at most 10 attachments.')
    await waitForUploads(10)
    expect(server.uploads.map((upload) => upload.filename)).not.toContain('huge.log')
  })

  it('keeps each conversation\'s attachments to itself', async () => {
    const user = userEvent.setup()
    const server = stubServer()
    renderChat('chat-a')

    fireEvent.paste(input(), { clipboardData: { files: [png()], types: ['Files'] } })
    await waitForUploads(1)

    await user.click(await screen.findByRole('button', { name: /Open thread Chat B/i }))
    await waitFor(() => expect(pendingAttachments()).toHaveLength(0))
    await user.type(input(), 'Only text here')
    await user.click(sendButton())
    await waitFor(() => expect(server.turns).toHaveLength(1))
    expect(server.turns[0]).toEqual({
      conversationId: 'chat-b',
      body: { project_path: PROJECT, message: 'Only text here', chat_mode: null },
    })

    await user.click(screen.getByRole('button', { name: /Open thread Chat A/i }))
    await waitForUploads(1)
    expect(server.uploads[0]!.conversationId).toBe('chat-a')
  })
})
