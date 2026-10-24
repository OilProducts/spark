import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { OrgPanel } from '../OrgPanel'
import { askCoordinatorDraft, renderableMarkdown } from '../orgModel'

const coordinator = {
    name: 'coordinator', title: 'Coordinator', state: 'idle', conversation_id: 'role-p1-coordinator',
    budget: { concurrent_runs: 4, runs_per_day: 20 }, recent_launches: [], runs: [],
    recent_events: [{ seq: 2, id: 'e2', at: '2026-10-24T10:00:00Z', kind: 'run_completed', source: 'run-1' }],
}
const items = [
    { kind: 'flow', name: 'shared', layer: 'project_copy', path: 'flows/shared', uncommitted: false },
    { kind: 'flow', name: 'local', layer: 'project', path: 'flows/local', uncommitted: true },
    { kind: 'role', name: 'reviewer', layer: 'bundled', path: 'roles/reviewer.md', uncommitted: false, usage: [{ flow: 'local', node: 'review' }] },
    { kind: 'role', name: 'coordinator', layer: 'bundled', path: 'roles/coordinator.md', uncommitted: false, usage: [], standing: coordinator },
]
const commits = [{ commit: 'abcdef1234567890', author: 'CoS', date: '2026-10-24T09:00:00Z', message: 'Copy shared from bundled', diff: 'diff --git a/flows/shared/flow.md b/flows/shared/flow.md\n+title: shared' }]
let urls: string[]

beforeEach(() => {
    window.innerWidth = 1440
    urls = []
    useStore.setState({ activeProjectPath: '/project', viewMode: 'org', projectRegistry: {
        '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-10-24T10:00:00Z' },
    } })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        urls.push(url)
        expect(init?.method ?? 'GET').toBe('GET')
        const path = url.split('?')[0]
        const params = new URL(url, 'http://spark').searchParams
        const json = (value: unknown) => ({ ok: true, json: async () => value })
        if (path === '/workspace/api/org') return json({ org_path: '/project/.spark/agent-org', bundled_path: '/bundled', org_exists: true, items })
        if (path === '/workspace/api/org/history') return json({ commits: params.get('path') === 'flows/shared' ? commits : [] })
        if (path === '/workspace/api/org/memory') return json(params.get('file') ? { content: '# Known\n\nThe build is slow.' } : { files: ['index.md'] })
        if (path === '/workspace/api/org/item') {
            const item = items.find((entry) => entry.name === params.get('name'))!
            return json({ ...item, files: [{ name: 'flow.md', content: '---\ntype: Flow\n---\nBody text', status: null }], bundled_diff: item.layer === 'project_copy' ? '-title: Shared v2\n+title: shared' : null, bundled_changed: item.layer === 'project_copy' ? true : null })
        }
        return json({})
    }))
})
afterEach(() => { vi.unstubAllGlobals() })

const row = (name: string) => screen.getAllByTestId('org-item').find((button) => button.textContent?.includes(name))!

it('groups items by layer with a badge, marking uncommitted ones', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Project copies' })
    expect(within(screen.getByRole('region', { name: 'Project copies' })).getByTestId('org-layer-badge')).toHaveTextContent('project copy')
    expect(within(screen.getByRole('region', { name: 'Project' })).getByTestId('org-item')).toHaveTextContent('uncommitted')
    expect(within(screen.getByRole('region', { name: 'Bundled' })).getAllByTestId('org-layer-badge').map((badge) => badge.textContent)).toEqual(['bundled', 'bundled'])
    expect(urls[0]).toBe('/workspace/api/org?project_path=%2Fproject')
})

it('shows a copy with its bundled diff, rendered content and history with diffs', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Project copies' })
    fireEvent.click(row('shared'))
    expect(await screen.findByTestId('org-bundled-changed')).toHaveTextContent('Bundled has changed since this copy was made.')
    const commit = await screen.findByTestId('org-commit')
    expect(commit).toHaveTextContent('Copy shared from bundled')
    expect(commit).toHaveTextContent('CoS')
    expect(commit).toHaveTextContent('abcdef12')
    expect(within(commit).getByTestId('org-diff')).toHaveTextContent('+title: shared')
    expect(within(screen.getByTestId('org-file')).getByText('Body text')).toBeInTheDocument()
})

it('shows where a role is used and whether it is standing, with its memory', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Bundled' })
    fireEvent.click(row('reviewer'))
    expect(await screen.findByTestId('org-role-usage')).toHaveTextContent('local · node review')
    expect(screen.getByTestId('org-role-standing')).toHaveTextContent('Not standing in this project.')
    fireEvent.click(row('coordinator'))
    expect(await screen.findByTestId('org-role-standing')).toHaveTextContent('Standing in this project (idle).')
    expect(screen.getByText(/0\/4 concurrent · 0\/20 today/)).toBeInTheDocument()
    expect(screen.getByText(/run_completed · run-1/)).toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: 'index.md' }))
    expect(await screen.findByTestId('org-memory-file')).toHaveTextContent('The build is slow.')
})

it('Ask the Coordinator opens its chat with an unsent draft naming the item', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Project copies' })
    fireEvent.click(row('shared'))
    fireEvent.click(await screen.findByRole('button', { name: 'Ask the Coordinator' }))
    await waitFor(() => expect(useStore.getState().viewMode).toBe('home'))
    const state = useStore.getState()
    expect(state.projectSessionsByPath['/project'].conversationId).toBe('role-p1-coordinator')
    expect(state.homeProjectSessionsByPath['/project'].chatDraft).toBe('About the flow "shared" (project copy, flows/shared): ')
    expect(urls.some((url) => url.includes('/turns'))).toBe(false)
})

it('builds drafts and renders frontmatter as YAML', () => {
    expect(askCoordinatorDraft({ kind: 'role', name: 'reviewer', layer: 'bundled', path: 'roles/reviewer.md' })).toBe('About the role "reviewer" (bundled, roles/reviewer.md): ')
    expect(renderableMarkdown('---\ntype: Role\n---\nPersona.')).toBe('```yaml\ntype: Role\n```\n\nPersona.')
    expect(renderableMarkdown('No frontmatter')).toBe('No frontmatter')
})
