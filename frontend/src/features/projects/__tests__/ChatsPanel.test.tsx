import { ActivityBar } from '@/app/ActivityBar'
import { HomeSessionController } from '@/app/AppSessionControllers'
import { DialogProvider } from '@/components/app/dialog-controller'
import { ProjectsPanel } from '@/features/projects/ProjectsPanel'
import { useStore } from '@/store'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const summary = (projectPath: string, id: string, updatedAt: string) => ({
    conversation_id: id, project_path: projectPath, title: `Chat ${id}`,
    created_at: updatedAt, updated_at: updatedAt, revision: 1, last_message_preview: null,
})
const snapshot = (projectPath: string, id: string) => ({
    schema_version: 5, revision: 1, conversation_id: id, project_path: projectPath, title: 'New thread',
    created_at: '2026-09-30T00:00:00Z', updated_at: '2026-09-30T00:00:00Z', turns: [], segments: [], event_log: [],
})

const HOME = '/home/me'
const chatsByProject: Record<string, ReturnType<typeof summary>[]> = {
    [HOME]: [summary(HOME, 'home-1', '2026-09-01T00:00:00Z')],
    '/work/busy': Array.from({ length: 7 }, (_, index) => summary('/work/busy', `busy-${index}`, `2026-09-1${index}T00:00:00Z`)).reverse(),
    '/work/gone': [],
}
let settingsWrites: { url: string; body: Record<string, unknown> }[]
let deletedIds: Set<string>

beforeEach(() => {
    window.innerWidth = 1280
    settingsWrites = []
    deletedIds = new Set()
    useStore.setState({
        viewMode: 'home', activeProjectPath: null, projectPagePath: null,
        homeConversationCache: { conversationsById: {}, summariesByProjectPath: {} },
        homeThreadSummariesStatusByProjectPath: {}, projectSessionsByPath: {}, homeProjectSessionsByPath: {},
        projectRegistry: {
            '/work/gone': { directoryPath: '/work/gone', isFavorite: false, lastAccessedAt: '2026-09-30T00:00:00Z', folderExists: false },
            [HOME]: { directoryPath: HOME, displayName: 'Home', isDefault: true, isFavorite: false, lastAccessedAt: null },
            '/work/busy': { directoryPath: '/work/busy', isFavorite: false, lastAccessedAt: '2026-09-20T00:00:00Z' },
        },
    })
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input)
        if (url.includes('/projects/conversations')) {
            return json((chatsByProject[new URL(url, 'http://localhost').searchParams.get('project_path') ?? ''] ?? []).filter((chat) => !deletedIds.has(chat.conversation_id)))
        }
        const conversationId = decodeURIComponent(url.match(/\/api\/conversations\/([^/?]+)/)?.[1] ?? '')
        if (conversationId && init?.method === 'DELETE') {
            deletedIds.add(conversationId)
            return json({ status: 'deleted', conversation_id: conversationId, project_path: '/work/busy' })
        }
        if (conversationId && init?.method === 'PUT') {
            const body = JSON.parse(String(init.body))
            settingsWrites.push({ url, body })
            return json(snapshot(String(body.project_path), conversationId))
        }
        if (conversationId) return json(snapshot(conversationId.startsWith('home') ? HOME : '/work/busy', conversationId))
        return json({})
    }))
})
afterEach(() => vi.unstubAllGlobals())

const renderChats = () => render(<DialogProvider><HomeSessionController /><ProjectsPanel /></DialogProvider>)
const group = (projectPath: string) => screen.getAllByTestId('chats-project-group').find((entry) => entry.dataset.projectPath === projectPath)!

it('groups chats by project with Home first and open, others by recent use and collapsed', async () => {
    renderChats()
    expect(screen.getAllByTestId('chats-project-group').map((entry) => entry.dataset.projectPath)).toEqual([HOME, '/work/gone', '/work/busy'])
    expect(await within(group(HOME)).findByRole('button', { name: 'Open thread Chat home-1' })).toBeVisible()
    expect(within(group('/work/busy')).getByTestId('chats-project-toggle')).toHaveAttribute('aria-expanded', 'false')
    expect(within(group('/work/busy')).queryByTestId('chats-chat-row')).not.toBeInTheDocument()
})

it('shows a project\'s recent chats with a "more" link to its project page', async () => {
    const user = userEvent.setup()
    renderChats()
    await user.click(within(group('/work/busy')).getByTestId('chats-project-toggle'))
    await waitFor(() => expect(within(group('/work/busy')).getAllByTestId('chats-chat-row')).toHaveLength(5))
    expect(within(group('/work/busy')).getAllByTestId('chats-chat-row')[0]).toHaveTextContent('Chat busy-6')
    await user.click(within(group('/work/busy')).getByTestId('chats-project-more'))
    expect(useStore.getState().projectPagePath).toBe('/work/busy')
    const page = screen.getByTestId('project-page')
    expect(within(page).getByTestId('project-page-path')).toHaveTextContent('/work/busy')
    expect(within(page).getAllByTestId('project-page-chat')).toHaveLength(7)
    expect(within(page).getByTestId('project-settings-dialog')).toBeInTheDocument()
    expect(within(page).getByRole('heading', { name: 'Project model defaults' })).toBeInTheDocument()
    expect(within(page).getByTestId('project-page-remove')).toBeEnabled()
})

it('marks a project whose folder is missing and offers no new chat there', () => {
    renderChats()
    expect(group('/work/gone')).toHaveAttribute('data-folder-missing', 'true')
    expect(within(group('/work/gone')).getByTestId('chats-project-missing')).toHaveTextContent('folder missing')
    expect(within(group('/work/gone')).queryByTestId('chats-project-new-chat')).not.toBeInTheDocument()
    expect(within(group(HOME)).queryByTestId('chats-project-missing')).not.toBeInTheDocument()
})

it('starts a chat in the project whose "+" was used and names that project in the composer', async () => {
    const user = userEvent.setup()
    renderChats()
    await user.click(within(group('/work/busy')).getByRole('button', { name: 'New chat in busy' }))
    await waitFor(() => expect(settingsWrites).toHaveLength(1))
    expect(settingsWrites[0].body).toMatchObject({ project_path: '/work/busy', expected_revision: '0' })
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe('/work/busy'))
    expect(within(group('/work/busy')).getByRole('button', { name: 'Open thread New thread' })).toHaveAttribute('aria-current', 'true')
    expect(screen.getByTestId('chat-composer-project')).toHaveTextContent('Runs and missions started here go to busy')
})

it('asks which project "+ New" starts a chat in, Home first then recent projects', async () => {
    const user = userEvent.setup()
    renderChats()
    await user.click(screen.getByTestId('project-thread-new-button'))
    const items = await screen.findAllByTestId('project-picker-item')
    expect(items.map((item) => item.dataset.projectPath)).toEqual([HOME, '/work/gone', '/work/busy'])
    expect(items[1]).toHaveAttribute('data-disabled')
    await user.click(items[0])
    await waitFor(() => expect(settingsWrites.at(-1)?.body).toMatchObject({ project_path: HOME }))
})

it('opens Home\'s project page without a way to remove it', async () => {
    const user = userEvent.setup()
    renderChats()
    await user.click(within(group(HOME)).getByTestId('chats-project-name'))
    expect(screen.getByTestId('project-page-title')).toHaveTextContent('Home')
    expect(screen.getByTestId('project-page-remove')).toBeDisabled()
    expect(screen.getByTestId('project-page-remove-refused')).toBeVisible()
})

it('keeps text typed into the restored chat before the project registry loads, and prunes removed projects once it has', async () => {
    const user = userEvent.setup()
    const registry = useStore.getState().projectRegistry
    useStore.setState({ projectRegistry: {}, activeProjectPath: '/work/busy', homeProjectSessionsByPath: { '/work/removed': { chatDraft: 'stale', panelError: null, pendingConversationTurn: null, pendingDeleteConversationId: null } } })
    renderChats()
    await user.type(screen.getByTestId('project-ai-conversation-input'), 'Typed early.')
    expect(screen.getByTestId('project-ai-conversation-input')).toHaveValue('Typed early.')
    expect(useStore.getState().homeProjectSessionsByPath['/work/removed']).toBeDefined()

    act(() => useStore.setState({ projectRegistry: registry }))
    expect(screen.getByTestId('project-ai-conversation-input')).toHaveValue('Typed early.')
    await waitFor(() => expect(useStore.getState().homeProjectSessionsByPath['/work/removed']).toBeUndefined())
})

it.each([
    ['empties another chat of the first project after visiting a second project', 'busy-5', ''],
    ['keeps a chat\'s draft when returning to it from another project', 'busy-6', 'Draft for busy-6'],
])('%s', async (_name, returnTo, expectedDraft) => {
    const user = userEvent.setup()
    renderChats()
    await user.click(within(group('/work/busy')).getByTestId('chats-project-toggle'))
    await user.click(await within(group('/work/busy')).findByRole('button', { name: 'Open thread Chat busy-6' }))
    const input = () => screen.getByTestId('project-ai-conversation-input')
    await waitFor(() => expect(input()).toBeEnabled())
    await user.type(input(), 'Draft for busy-6')
    await user.click(await within(group(HOME)).findByRole('button', { name: 'Open thread Chat home-1' }))
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe(HOME))
    expect(input()).toHaveValue('')
    await user.click(within(group('/work/busy')).getAllByTestId('chats-chat-row').find((row) => row.dataset.conversationId === returnTo)!)
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe('/work/busy'))
    expect(input()).toHaveValue(expectedDraft)
})

it('empties the fallback chat after deleting a project\'s drafted chat while another project is shown', async () => {
    const user = userEvent.setup()
    renderChats()
    await user.click(within(group('/work/busy')).getByTestId('chats-project-toggle'))
    await user.click(await within(group('/work/busy')).findByRole('button', { name: 'Open thread Chat busy-6' }))
    const input = () => screen.getByTestId('project-ai-conversation-input')
    await waitFor(() => expect(input()).toBeEnabled())
    await user.type(input(), 'Draft for busy-6')
    await user.click(await within(group(HOME)).findByRole('button', { name: 'Open thread Chat home-1' }))
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe(HOME))
    await user.click(within(group('/work/busy')).getByTestId('project-thread-delete-busy-6'))
    await user.click(await screen.findByRole('button', { name: 'Delete thread' }))
    await waitFor(() => expect(useStore.getState().projectSessionsByPath['/work/busy']?.conversationId).not.toBe('busy-6'))
    const fallback = useStore.getState().projectSessionsByPath['/work/busy']!.conversationId!
    await user.click(within(group('/work/busy')).getAllByTestId('chats-chat-row').find((row) => row.dataset.conversationId === fallback)!)
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe('/work/busy'))
    expect(input()).toHaveValue('')
})

it.each([
    ['empties another chat of a project opened from a notification after visiting a second project', 'busy-5', ''],
    ['keeps a chat\'s draft when a notification reopens that same chat', 'busy-6', 'Draft for busy-6'],
])('%s', async (_name, notifiedChat, expectedDraft) => {
    const user = userEvent.setup()
    vi.mocked(fetch).mockImplementation(((original) => async (input: RequestInfo | URL, init?: RequestInit) => {
        if (String(input).includes('/attention')) {
            return json({ items: [{ kind: 'proposed_plan', id: 'plan', title: 'Plan', project_path: '/work/busy', conversation_id: notifiedChat }] })
        }
        return original(input, init)
    })(vi.mocked(fetch).getMockImplementation()!))
    render(<DialogProvider><HomeSessionController /><ActivityBar /><ProjectsPanel /></DialogProvider>)
    await user.click(within(group('/work/busy')).getByTestId('chats-project-toggle'))
    await user.click(await within(group('/work/busy')).findByRole('button', { name: 'Open thread Chat busy-6' }))
    const input = () => screen.getByTestId('project-ai-conversation-input')
    await waitFor(() => expect(input()).toBeEnabled())
    await user.type(input(), 'Draft for busy-6')
    await user.click(await within(group(HOME)).findByRole('button', { name: 'Open thread Chat home-1' }))
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe(HOME))
    await user.click(screen.getByTestId('attention-bell'))
    await user.click(await screen.findByTestId('attention-bell-item'))
    await waitFor(() => expect(useStore.getState().activeProjectPath).toBe('/work/busy'))
    expect(useStore.getState().projectSessionsByPath['/work/busy']?.conversationId).toBe(notifiedChat)
    expect(input()).toHaveValue(expectedDraft)
})
