import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { RolesPanel } from '../RolesPanel'
import { definitionBadges, frontmatterFields, renderableMarkdown, roleAction, usageSummary } from '../rolesModel'

const live = (name: string, handle: string | null) => ({
    name, title: name, state: 'idle', conversation_id: `role-p1-${name}`, conversation_handle: handle,
    budget: { concurrent_runs: 4 }, runs: [],
    recent_events: [{ seq: 2, id: 'e2', at: '2026-10-24T10:00:00Z', kind: 'run_completed', source: 'run-1' }],
})
const role = (name: string, fields: Record<string, unknown> = {}) =>
    ({ kind: 'role', name, layer: 'bundled', path: `roles/${name}.md`, usage: [], standing: false, live: null, ...fields })
const items = [
    { kind: 'flow', name: 'local', layer: 'project', path: 'flows/local' },
    role('reviewer', { title: 'Reviewer', description: 'Reviews work.',
        usage: [{ flow: 'local', node: 'review' }, { flow: 'other', node: 'check' }, { flow: 'local', node: 'recheck' }] }),
    role('helper', { layer: 'project' }),
    role('author', { layer: 'project_copy' }),
    role('keeper', { title: 'Keeper', standing: true, live: live('keeper', 'keeper-1') }),
    // A saved record whose conversation is gone.
    role('lapsed', { title: 'Lapsed', standing: true, live: live('lapsed', null) }),
    role('watcher', { title: 'Watcher', description: 'Watches.', layer: 'project', standing: true }),
]
const lists: Record<string, unknown[]> = { '/project': items, '/other': [role('watcher', { title: 'Other watcher', standing: true })] }

type Settle = { resolve: (value: unknown) => void; reject: (error: Error) => void }
let calls: { url: string; method: string }[]
let starts: Settle[]

beforeEach(() => {
    window.innerWidth = 1440
    calls = []
    starts = []
    useStore.setState({ activeProjectPath: '/project', viewMode: 'roles', projectPagePath: null, projectRegistry: {
        '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-10-24T10:00:00Z' },
        '/other': { directoryPath: '/other', isFavorite: false, lastAccessedAt: '2026-10-23T10:00:00Z' },
    } })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        const method = init?.method ?? 'GET'
        calls.push({ url, method })
        const path = url.split('?')[0]
        const params = new URL(url, 'http://spark').searchParams
        const json = (value: unknown) => ({ ok: true, json: async () => value })
        if (method === 'POST') return new Promise((resolve, reject) => starts.push({ resolve, reject }))
        if (path === '/workspace/api/definitions') return json({ items: lists[params.get('project_path')!] })
        const route = /^\/workspace\/api\/roles\/([^/]+)\/(file|memory)$/.exec(path)
        if (route?.[2] === 'memory') return json(params.get('file') ? { role: route[1], file: 'index.md', content: '# Known\n\nThe build is slow.' } : { role: route[1], files: ['index.md'] })
        if (route?.[2] === 'file') {
            const item = items.find((entry) => entry.name === route[1])!
            return json({ ...item, files: [{ name: `${item.name}.md`, content: '---\ntype: Role\ntitle: Author\n---\nYou **write**.' }], bundled_diff: item.layer === 'project_copy' ? '-title: Author v2\n+title: Author' : null })
        }
        return json({})
    }))
})
afterEach(() => { vi.unstubAllGlobals() })

const row = (name: string) => screen.getAllByTestId('roles-item').find((button) => button.dataset.name === name)!
const badges = (name: string) => within(row(name)).queryAllByTestId('definition-badge').map((badge) => badge.textContent)
const detail = () => within(screen.getByRole('region', { name: 'Role' }))
const listFetches = (project: string) => calls.filter((call) => call.url === `/workspace/api/definitions?project_path=${encodeURIComponent(project)}`).length
const conversation = () => useStore.getState().projectSessionsByPath['/project']?.conversationId
const respond = (status: number, body: unknown) => ({ ok: status < 400, status, json: async () => body })

it('groups roles by their file\'s standing flag, started or not, with badges and no status line', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    expect(screen.getAllByTestId('roles-item').map((button) => button.dataset.name)).toEqual(['keeper', 'lapsed', 'watcher', 'reviewer', 'helper', 'author'])
    expect(within(screen.getByRole('region', { name: 'Standing' })).getAllByTestId('roles-item')).toHaveLength(3)
    expect(screen.queryByText('local')).not.toBeInTheDocument()
    expect(row('keeper')).toHaveTextContent('idle · 0/4 concurrent')
    expect(row('keeper')).toHaveTextContent('Last wake run_completed')
    expect(row('watcher')).toHaveTextContent('Watches.')
    expect(row('reviewer')).toHaveTextContent('used in 3 steps across 2 flows')
    expect([badges('reviewer'), badges('helper'), badges('author')]).toEqual([[], ['customized'], ['project copy']])
    // Open for a conversation, Start for a standing role without one, nothing for the rest.
    expect(screen.getByRole('button', { name: 'Open conversation with Keeper' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Start Lapsed' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Start Watcher' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^(Start|Open conversation with) (Reviewer|helper|author)$/ })).not.toBeInTheDocument()
    expect(screen.getByTestId('roles-empty-state')).toHaveClass('justify-center')
    expect(calls[0].url).toBe('/workspace/api/definitions?project_path=%2Fproject')
})

it('shows a copy with its prompt and bundled diff', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Roles' })
    fireEvent.click(row('author'))
    expect(await within(screen.getByRole('region', { name: 'Diff from bundled' })).findByTestId('roles-diff')).toHaveTextContent('-title: Author v2')
    const frontmatter = await screen.findByTestId('role-frontmatter')
    expect(within(frontmatter).getByText('title').nextSibling).toHaveTextContent('Author')
    expect(within(screen.getByTestId('role-file')).getByText('write').tagName).toBe('STRONG')
    expect(calls.some((call) => call.url === '/workspace/api/roles/author/file?project_path=%2Fproject')).toBe(true)
    expect(detail().queryByRole('button', { name: /Start|Open conversation/ })).not.toBeInTheDocument()
})

it('groups usage by flow and shows a started role\'s live status once, with its memory', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Roles' })
    fireEvent.click(row('reviewer'))
    const usage = await screen.findByTestId('role-usage')
    expect(within(usage).getAllByRole('listitem').map((item) => item.textContent)).toEqual(['local · review, recheck', 'other · check'])
    expect(screen.queryByTestId('role-live-status')).not.toBeInTheDocument()
    fireEvent.click(row('keeper'))
    expect(await screen.findAllByTestId('role-live-status')).toHaveLength(1)
    expect(screen.getByText(/run_completed · run-1/)).toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: 'index.md' }))
    expect(await screen.findByTestId('role-memory-file')).toHaveTextContent('The build is slow.')
    expect(calls.some((call) => call.url === '/workspace/api/roles/keeper/memory?project_path=%2Fproject&file=index.md')).toBe(true)
})

it('opens an existing conversation from the row and the detail without starting', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(screen.getByRole('button', { name: 'Open conversation with Keeper' }))
    expect(conversation()).toBe('role-p1-keeper')
    expect(useStore.getState().viewMode).toBe('home')
    useStore.getState().updateProjectSessionState('/project', { conversationId: null })
    fireEvent.click(row('keeper'))
    fireEvent.click(detail().getByRole('button', { name: 'Open conversation' }))
    expect(conversation()).toBe('role-p1-keeper')
    await screen.findByTestId('role-file')
    await screen.findByRole('button', { name: 'index.md' })
    expect(calls.some((call) => call.method === 'POST')).toBe(false)
})

it('starts from the row once, then opens the returned conversation and refreshes', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(row('watcher'))
    fireEvent.click(screen.getByRole('button', { name: 'Start Watcher' }))
    // Both entry points wait for the start.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Start Watcher' })).toBeDisabled())
    expect(detail().getByRole('button', { name: 'Starting…' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Start Watcher' }))
    expect(calls.filter((call) => call.method === 'POST').map((call) => call.url)).toEqual(['/workspace/api/roles/watcher/start?project_path=%2Fproject'])
    expect(listFetches('/project')).toBe(1)
    starts[0].resolve(respond(200, { ...live('watcher', 'watcher-1'), conversation_id: 'role-p1-watcher' }))
    await waitFor(() => expect(conversation()).toBe('role-p1-watcher'))
    expect(useStore.getState().viewMode).toBe('home')
    await waitFor(() => expect(listFetches('/project')).toBe(2))
    expect(screen.getByRole('button', { name: 'Start Watcher' })).toBeEnabled()
})

it('starts a saved record without a conversation from the detail', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(row('lapsed'))
    fireEvent.click(detail().getByRole('button', { name: 'Start' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Start Lapsed' })).toBeDisabled())
    starts[0].resolve(respond(200, { ...live('lapsed', 'lapsed-1') }))
    await waitFor(() => expect(conversation()).toBe('role-p1-lapsed'))
    expect(calls.find((call) => call.method === 'POST')?.url).toBe('/workspace/api/roles/lapsed/start?project_path=%2Fproject')
})

it('shows a failed start inline in the row and the detail, and allows a retry', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(row('watcher'))
    fireEvent.click(detail().getByRole('button', { name: 'Start' }))
    await waitFor(() => expect(starts).toHaveLength(1))
    starts[0].resolve(respond(409, { detail: 'watcher cannot start' }))
    expect(await screen.findAllByText('watcher cannot start')).toHaveLength(2)
    expect(useStore.getState().viewMode).toBe('roles')
    expect(listFetches('/project')).toBe(1)
    fireEvent.click(screen.getByRole('button', { name: 'Start Watcher' }))
    await waitFor(() => expect(starts).toHaveLength(2))
    expect(screen.queryByText('watcher cannot start')).not.toBeInTheDocument()
    starts[1].resolve(respond(200, { ...live('watcher', 'watcher-1') }))
    await waitFor(() => expect(conversation()).toBe('role-p1-watcher'))
})

it('a start that finishes after a project switch opens its own chat and leaves the new list alone', async () => {
    render(<RolesPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(screen.getByRole('button', { name: 'Start Watcher' }))
    await waitFor(() => expect(starts).toHaveLength(1))
    fireEvent.keyDown(screen.getByRole('button', { name: 'project' }), { key: 'Enter' })
    fireEvent.click(screen.getAllByTestId('project-picker-item').find((item) => item.dataset.projectPath === '/other')!)
    await screen.findByText('Other watcher')
    // The other project's role of the same name is not the one starting.
    expect(screen.getByRole('button', { name: 'Start Other watcher' })).toBeEnabled()
    starts[0].resolve(respond(200, { ...live('watcher', 'watcher-1') }))
    await waitFor(() => expect(conversation()).toBe('role-p1-watcher'))
    expect(useStore.getState().activeProjectPath).toBe('/project')
    expect(screen.getAllByTestId('roles-item').map((button) => button.textContent)).toEqual([expect.stringContaining('Other watcher')])
    expect(listFetches('/project')).toBe(1)
    expect(listFetches('/other')).toBe(1)
})

it('builds actions, badges, usage summaries and frontmatter fields', () => {
    expect(roleAction({ standing: true, live: null })).toBe('start')
    expect(roleAction({ standing: true, live: { ...live('a', null), state: 'idle' } })).toBe('start')
    expect(roleAction({ standing: true, live: { ...live('a', 'a-1'), state: 'idle' } })).toBe('open')
    expect(roleAction({ standing: false, live: null })).toBe(null)
    expect(renderableMarkdown('---\ntype: Role\n---\nPersona.')).toBe('```yaml\ntype: Role\n```\n\nPersona.')
    expect(renderableMarkdown('No frontmatter')).toBe('No frontmatter')
    expect(definitionBadges({ layer: 'project_copy' })).toEqual(['project copy'])
    expect(definitionBadges({ layer: 'bundled' })).toEqual([])
    expect(usageSummary([{ flow: 'a', node: 'x' }])).toBe('used in 1 step across 1 flow')
    expect(frontmatterFields('---\ntype: Role\nbudget:\n  concurrent_runs: 3\n---\nBody')).toEqual({ fields: [['type', 'Role'], ['budget', 'concurrent_runs: 3']], body: 'Body' })
})
