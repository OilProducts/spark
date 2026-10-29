import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { OrgPanel } from '../OrgPanel'
import { askCoordinatorDraft, frontmatterFields, orgBadges, renderableMarkdown, usageSummary } from '../orgModel'

const coordinator = {
    name: 'coordinator', title: 'Coordinator', state: 'idle', conversation_id: 'role-p1-coordinator',
    budget: { concurrent_runs: 4, runs_per_day: 20 }, recent_launches: [], runs: [],
    recent_events: [{ seq: 2, id: 'e2', at: '2026-10-24T10:00:00Z', kind: 'run_completed', source: 'run-1' }],
}
const items = [
    { kind: 'flow', name: 'local', layer: 'project', path: 'flows/local', uncommitted: true },
    { kind: 'role', name: 'reviewer', title: 'Reviewer', description: 'Reviews work.', layer: 'bundled', path: 'roles/reviewer.md', uncommitted: false,
        usage: [{ flow: 'local', node: 'review' }, { flow: 'other', node: 'check' }, { flow: 'local', node: 'recheck' }] },
    { kind: 'role', name: 'helper', layer: 'project', path: 'roles/helper.md', uncommitted: true, usage: [] },
    { kind: 'role', name: 'author', layer: 'project_copy', path: 'roles/author.md', uncommitted: false, bundled_changed: true, usage: [] },
    { kind: 'role', name: 'coordinator', title: 'Coordinator', layer: 'bundled', path: 'roles/coordinator.md', uncommitted: false, usage: [], standing: coordinator },
]
const commits = [{ commit: 'abcdef1234567890', author: 'CoS', date: '2026-10-24T09:00:00Z', message: 'Copy author from bundled', diff: 'diff --git a/roles/author.md b/roles/author.md\n+title: Author' }]
let urls: string[]
let orgExists: boolean

beforeEach(() => {
    window.innerWidth = 1440
    urls = []
    orgExists = true
    useStore.setState({ activeProjectPath: '/project', viewMode: 'org', projectRegistry: {
        '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-10-24T10:00:00Z' },
    } })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        urls.push(url)
        expect(init?.method ?? 'GET').toBe('GET')
        const path = url.split('?')[0]
        const params = new URL(url, 'http://spark').searchParams
        const json = (value: unknown) => ({ ok: true, json: async () => value })
        if (path === '/workspace/api/org') return json({ org_path: '/project/.spark/agent-org', bundled_path: '/bundled', org_exists: orgExists,
            items: orgExists ? items : items.map((item) => ({ ...item, layer: item.kind === 'role' ? 'bundled' : item.layer, uncommitted: false })) })
        if (path === '/workspace/api/org/history') return json({ commits: params.get('path') === 'roles/author.md' ? commits : [] })
        if (path === '/workspace/api/org/memory') return json(params.get('file') ? { content: '# Known\n\nThe build is slow.' } : { files: ['index.md'] })
        if (path === '/workspace/api/org/item') {
            const item = items.find((entry) => entry.name === params.get('name'))!
            return json({ ...item, files: [{ name: `${item.name}.md`, content: '---\ntype: Role\ntitle: Author\n---\nYou **write**.', status: null }], bundled_diff: item.layer === 'project_copy' ? '-title: Author v2\n+title: Author' : null })
        }
        return json({})
    }))
})
afterEach(() => { vi.unstubAllGlobals() })

const row = (name: string) => screen.getAllByTestId('org-item').find((button) => button.dataset.name === name)!
const badges = (name: string) => within(row(name)).queryAllByTestId('org-badge').map((badge) => badge.textContent)

it('lists roles only, standing first, with non-default badges and the org status', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    expect(screen.getByTestId('org-status')).toHaveTextContent('2 customized roles · 1 uncommitted')
    expect(screen.getAllByTestId('org-item').map((button) => button.dataset.name)).toEqual(['coordinator', 'reviewer', 'helper', 'author'])
    expect(screen.queryByText('local')).not.toBeInTheDocument()
    expect(row('coordinator')).toHaveTextContent('idle · 0/4 concurrent · 0/20 today')
    expect(row('coordinator')).toHaveTextContent('Last wake run_completed')
    expect(row('reviewer')).toHaveTextContent('Reviews work.')
    expect(row('reviewer')).toHaveTextContent('used in 3 steps across 2 flows')
    expect(badges('reviewer')).toEqual([])
    expect(badges('coordinator')).toEqual([])
    expect(badges('helper')).toEqual(['customized', 'uncommitted'])
    expect(badges('author')).toEqual(['project copy'])
    expect(screen.queryByText('bundled')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'project' })).toBeInTheDocument()
    expect(urls[0]).toBe('/workspace/api/org?project_path=%2Fproject')
})

it('says all roles are bundled defaults without an org repo', async () => {
    orgExists = false
    render(<OrgPanel />)
    expect(await screen.findByTestId('org-status')).toHaveTextContent('No org repo: all roles are bundled defaults.')
    expect(screen.queryAllByTestId('org-badge')).toEqual([])
})

it('shows a copy with its prompt, bundled diff and collapsed history', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Roles' })
    expect(screen.getByTestId('org-empty-state')).toHaveClass('justify-center')
    fireEvent.click(row('author'))
    expect(await screen.findByTestId('org-bundled-changed')).toHaveTextContent('Bundled has changed since this copy was made.')
    expect(await within(screen.getByRole('region', { name: 'Diff from bundled' })).findByTestId('org-diff')).toHaveTextContent('-title: Author v2')
    const frontmatter = await screen.findByTestId('org-frontmatter')
    expect(within(frontmatter).getByText('title').nextSibling).toHaveTextContent('Author')
    expect(within(screen.getByTestId('org-file')).getByText('write').tagName).toBe('STRONG')
    const history = screen.getByText('History').closest('details')!
    expect(history.open).toBe(false)
    expect(await within(history).findByTestId('org-commit')).toHaveTextContent('Copy author from bundled')
})

it('groups usage by flow and shows a standing role\'s live status once, with its memory', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Roles' })
    fireEvent.click(row('reviewer'))
    const usage = await screen.findByTestId('org-role-usage')
    expect(within(usage).getAllByRole('listitem').map((item) => item.textContent)).toEqual(['local · review, recheck', 'other · check'])
    expect(screen.queryByTestId('org-live-status')).not.toBeInTheDocument()
    fireEvent.click(row('coordinator'))
    expect(await screen.findAllByTestId('org-live-status')).toHaveLength(1)
    expect(screen.getByText(/run_completed · run-1/)).toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: 'index.md' }))
    expect(await screen.findByTestId('org-memory-file')).toHaveTextContent('The build is slow.')
})

it('hides Ask the Coordinator on the coordinator itself', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Standing' })
    fireEvent.click(row('coordinator'))
    await screen.findByTestId('org-live-status')
    expect(screen.queryByRole('button', { name: 'Ask the Coordinator' })).not.toBeInTheDocument()
})

it('Ask the Coordinator opens its chat with an unsent draft naming the role', async () => {
    render(<OrgPanel />)
    await screen.findByRole('region', { name: 'Roles' })
    fireEvent.click(row('author'))
    fireEvent.click(await screen.findByRole('button', { name: 'Ask the Coordinator' }))
    await waitFor(() => expect(useStore.getState().viewMode).toBe('home'))
    const state = useStore.getState()
    expect(state.projectSessionsByPath['/project'].conversationId).toBe('role-p1-coordinator')
    expect(state.homeProjectSessionsByPath['/project'].chatDraft).toBe('About the role "author" (project copy, roles/author.md): ')
    expect(urls.some((url) => url.includes('/turns'))).toBe(false)
})

it('builds drafts, badges, usage summaries and frontmatter fields', () => {
    expect(askCoordinatorDraft({ kind: 'role', name: 'reviewer', layer: 'bundled', path: 'roles/reviewer.md' })).toBe('About the role "reviewer" (bundled, roles/reviewer.md): ')
    expect(renderableMarkdown('---\ntype: Role\n---\nPersona.')).toBe('```yaml\ntype: Role\n```\n\nPersona.')
    expect(renderableMarkdown('No frontmatter')).toBe('No frontmatter')
    expect(orgBadges({ layer: 'project_copy', uncommitted: false, bundled_changed: false })).toEqual([])
    expect(orgBadges({ layer: 'bundled', uncommitted: false })).toEqual([])
    expect(usageSummary([{ flow: 'a', node: 'x' }])).toBe('used in 1 step across 1 flow')
    expect(frontmatterFields('---\ntype: Role\nbudget:\n  runs_per_day: 3\n---\nBody')).toEqual({ fields: [['type', 'Role'], ['budget', 'runs_per_day: 3']], body: 'Body' })
})
