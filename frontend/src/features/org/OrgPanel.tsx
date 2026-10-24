import { useEffect, useState, type ReactNode } from 'react'
import { MessageSquare, PenLine, RefreshCw } from 'lucide-react'
import { useStore } from '@/store'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { ListGroupLabel, SidePanelRow } from '@/components/app/side-panel-row'
import { ViewLayout } from '@/components/app/view-layout'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { defaultProjectChoice, projectLabel } from '@/features/projects/model/projectChoices'
import { openChat } from '@/app/useAttention'
import { cn } from '@/lib/utils'
import {
    askCoordinatorDraft, budgetLine, LAYER_GROUPS, LAYER_LABELS, orgRequest, renderableMarkdown,
    type OrgCommit, type OrgItem, type OrgItemDetail, type OrgLayer, type OrgList, type StandingRole,
} from './orgModel'

export function LayerBadge({ layer }: { layer: OrgLayer }) {
    return <Badge data-testid="org-layer-badge" variant={layer === 'bundled' ? 'secondary' : 'default'}>{LAYER_LABELS[layer]}</Badge>
}

/** A unified diff with added and removed lines marked. */
export function DiffBlock({ diff }: { diff: string }) {
    if (!diff.trim()) return <p className="text-xs text-muted-foreground">No differences.</p>
    return <pre data-testid="org-diff" className="max-h-96 overflow-auto rounded-md border bg-muted/40 p-2 text-xs leading-5">
        {diff.split('\n').map((line, index) => <div key={index} className={cn(
            line.startsWith('+') && !line.startsWith('+++') && 'text-success',
            line.startsWith('-') && !line.startsWith('---') && 'text-destructive',
            line.startsWith('@@') && 'text-muted-foreground',
        )}>{line || ' '}</div>)}
    </pre>
}

function Section({ title, children }: { title: string; children: ReactNode }) {
    return <section aria-label={title} className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold">{title}</h3>
        {children}
    </section>
}

/** The org repo's commits for one path, each with its diff behind a disclosure. */
export function OrgHistory({ commits }: { commits: OrgCommit[] | null }) {
    if (!commits) return <p className="text-xs text-muted-foreground">Loading…</p>
    if (!commits.length) return <p className="text-xs text-muted-foreground">No commits in the org repo.</p>
    return <ul className="flex flex-col gap-2">{commits.map((commit) => <li key={commit.commit} data-testid="org-commit">
        <details className="rounded-md border p-2">
            <summary className="cursor-pointer text-sm">
                <span className="font-medium">{commit.message.split('\n')[0]}</span>
                <span className="text-xs text-muted-foreground"> · {commit.author} · {commit.date} · {commit.commit.slice(0, 8)}</span>
            </summary>
            {commit.message.includes('\n') && <p className="mt-2 whitespace-pre-wrap text-xs">{commit.message}</p>}
            <div className="mt-2"><DiffBlock diff={commit.diff} /></div>
        </details>
    </li>)}</ul>
}

/** Where a role is used and whether it is standing here. */
export function RoleUsage({ item }: { item: OrgItem }) {
    const usage = item.usage ?? []
    return <div className="flex flex-col gap-1 text-sm">
        {usage.length
            ? <ul data-testid="org-role-usage" className="flex flex-col gap-0.5">{usage.map((use) => <li key={`${use.flow}:${use.node}`}>{use.flow} · <span className="text-muted-foreground">node</span> {use.node}</li>)}</ul>
            : <p className="text-muted-foreground">No flow node uses this role.</p>}
        <p data-testid="org-role-standing">{item.standing ? `Standing in this project (${item.standing.state}).` : 'Not standing in this project.'}</p>
    </div>
}

function MemoryBrowser({ project, role }: { project: string; role: string }) {
    const [files, setFiles] = useState<string[] | null>(null)
    const [open, setOpen] = useState<{ file: string; content: string } | null>(null)
    const [error, setError] = useState('')
    useEffect(() => {
        orgRequest<{ files: string[] }>('/memory', project, { role }).then((value) => setFiles(value.files), (e: Error) => setError(e.message))
    }, [project, role])
    const read = (file: string) => orgRequest<{ content: string }>('/memory', project, { role, file })
        .then((value) => setOpen({ file, content: value.content }), (e: Error) => setError(e.message))
    if (error) return <InlineError dense>{error}</InlineError>
    if (!files) return <p className="text-xs text-muted-foreground">Loading…</p>
    if (!files.length) return <p className="text-xs text-muted-foreground">No memory files.</p>
    return <div className="flex flex-col gap-2">
        <ul className="flex flex-wrap gap-1">{files.map((file) => <li key={file}>
            <Button type="button" size="xs" variant={open?.file === file ? 'outline' : 'ghost'} aria-pressed={open?.file === file} onClick={() => void read(file)}>{file}</Button>
        </li>)}</ul>
        {open && <div data-testid="org-memory-file" className="rounded-md border p-3"><ProjectConversationMarkdown content={renderableMarkdown(open.content)} /></div>}
    </div>
}

function StandingDetails({ project, role }: { project: string; role: StandingRole }) {
    return <div className="flex flex-col gap-2 text-sm">
        <p><span className="text-muted-foreground">State</span> {role.state} · <span className="text-muted-foreground">Budget</span> {budgetLine(role)}</p>
        <Button type="button" size="sm" variant="outline" className="self-start" onClick={() => openChat(project, role.conversation_id)}>
            <MessageSquare aria-hidden="true" />Open conversation
        </Button>
        <h4 className="text-xs font-semibold text-muted-foreground">Recent wakes</h4>
        {role.recent_events.length
            ? <ul className="flex flex-col gap-0.5 text-xs">{role.recent_events.map((event) => <li key={event.seq}>{event.kind} · {event.source} · <span className="text-muted-foreground">{event.at}</span></li>)}</ul>
            : <p className="text-xs text-muted-foreground">No wakes yet.</p>}
        <h4 className="text-xs font-semibold text-muted-foreground">Memory</h4>
        <MemoryBrowser project={project} role={role.name} />
    </div>
}

function ItemDetail({ project, item, coordinator }: { project: string; item: OrgItem; coordinator: StandingRole | null }) {
    const [detail, setDetail] = useState<OrgItemDetail | null>(null)
    const [commits, setCommits] = useState<OrgCommit[] | null>(null)
    const [error, setError] = useState('')
    useEffect(() => {
        orgRequest<OrgItemDetail>('/item', project, { kind: item.kind, name: item.name }).then(setDetail, (e: Error) => setError(e.message))
        orgRequest<{ commits: OrgCommit[] }>('/history', project, { path: item.path }).then((value) => setCommits(value.commits), (e: Error) => setError(e.message))
    }, [project, item.kind, item.name, item.path])
    const openInEditor = () => {
        useStore.getState().setActiveProjectPath(project)
        useStore.getState().setActiveFlow(item.name)
        useStore.getState().setViewMode('editor')
    }
    return <section aria-label="Org item" className="flex h-full min-h-0 flex-col gap-4 overflow-y-auto p-4">
        <header className="flex flex-wrap items-center gap-2">
            <h2 className="text-base font-semibold">{item.name}</h2>
            <span className="text-xs text-muted-foreground">{item.kind}</span>
            <LayerBadge layer={item.layer} />
            {item.uncommitted && <Badge variant="destructive">uncommitted</Badge>}
            <span className="flex-1" />
            {item.kind === 'flow' && <Button type="button" size="sm" variant="ghost" onClick={openInEditor}><PenLine aria-hidden="true" />Open in editor</Button>}
            <Button type="button" size="sm" variant="outline" disabled={!coordinator}
                onClick={() => coordinator && openChat(project, coordinator.conversation_id, askCoordinatorDraft(item))}>
                <MessageSquare aria-hidden="true" />Ask the Coordinator
            </Button>
        </header>
        <p className="text-xs text-muted-foreground">{item.path}</p>
        {error && <InlineError dense>{error}</InlineError>}
        {item.kind === 'role' && <Section title="Usage"><RoleUsage item={item} /></Section>}
        {item.standing && <Section title="Standing role"><StandingDetails project={project} role={item.standing} /></Section>}
        {detail?.layer === 'project_copy' && <Section title="Copy of bundled">
            <p data-testid="org-bundled-changed" className="text-sm">{detail.bundled_changed === null
                ? 'The copy is not committed yet, so its bundled origin is unknown.'
                : detail.bundled_changed ? 'Bundled has changed since this copy was made.' : 'Bundled is unchanged since this copy was made.'}</p>
            <p className="text-xs text-muted-foreground">Diff from today's bundled version to the copy:</p>
            <DiffBlock diff={detail.bundled_diff ?? ''} />
        </Section>}
        <Section title="Content">
            {detail ? detail.files.map((file) => <div key={file.name} data-testid="org-file" className="rounded-md border p-3">
                <div className="mb-2 flex items-center gap-2 text-xs font-medium">{file.name}{file.status && <Badge variant="destructive">{file.status}</Badge>}</div>
                <ProjectConversationMarkdown content={renderableMarkdown(file.content)} />
            </div>) : !error && <p className="text-xs text-muted-foreground">Loading…</p>}
        </Section>
        <Section title="History"><OrgHistory commits={commits} /></Section>
    </section>
}

/** A read-only view of a project's flows and roles by layer; changes go through the Coordinator. */
export function OrgPanel() {
    const registry = useStore((state) => state.projectRegistry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const [project, setProject] = useState(() => activeProjectPath ?? defaultProjectChoice(registry))
    const [list, setList] = useState<OrgList | null>(null)
    const [selected, setSelected] = useState<{ kind: string; name: string } | null>(null)
    const [error, setError] = useState('')
    const [refreshes, setRefreshes] = useState(0)
    useEffect(() => {
        if (!project) return
        let current = true
        setError('')
        orgRequest<OrgList>('', project).then((value) => current && setList(value), (e: Error) => current && setError(e.message))
        return () => { current = false }
    }, [project, refreshes])
    const items = list?.items ?? []
    const item = items.find((entry) => entry.kind === selected?.kind && entry.name === selected?.name) ?? null
    const coordinator = items.find((entry) => entry.kind === 'role' && entry.name === 'coordinator')?.standing ?? null
    const panel = <div className="flex flex-col gap-3">
        <p className="px-3 text-xs text-muted-foreground" title={project ?? undefined}>{projectLabel(registry, project)}</p>
        {error && <InlineError>{error}</InlineError>}
        {LAYER_GROUPS.map(([layer, label]) => {
            const grouped = items.filter((entry) => entry.layer === layer)
            if (!grouped.length) return null
            return <section key={layer} aria-label={label} className="flex flex-col gap-0.5">
                <ListGroupLabel><h3>{label}</h3></ListGroupLabel>
                <ul className="flex flex-col gap-0.5">{grouped.map((entry) => <li key={`${entry.kind}:${entry.name}`}>
                    <SidePanelRow title={entry.name} data-testid="org-item" data-kind={entry.kind}
                        selected={item === entry}
                        meta={<>{entry.kind}{entry.uncommitted ? ' · uncommitted' : ''}{entry.standing ? ' · standing' : ''}</>}
                        trailing={<LayerBadge layer={entry.layer} />}
                        onClick={() => setSelected({ kind: entry.kind, name: entry.name })} />
                </li>)}</ul>
            </section>
        })}
        {!project && <Empty className="text-xs"><EmptyDescription>Choose a project</EmptyDescription></Empty>}
        {project && !list && !error && <p className="px-3 py-2 text-sm text-muted-foreground">Loading…</p>}
    </div>
    return <ViewLayout view="org" title="Org" panel={panel} actions={<>
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh" onClick={() => setRefreshes((value) => value + 1)}><RefreshCw aria-hidden="true" className="size-4" /></Button>
        <ProjectPicker heading="Show the org of" testId="org-project-picker" defaultProjectPath={project} onPick={(path) => { setProject(path); setSelected(null); setList(null) }}>
            <Button type="button" variant="outline" size="sm">Project</Button>
        </ProjectPicker>
    </>}>
        {project && item
            ? <ItemDetail key={`${project}:${item.kind}:${item.name}:${refreshes}`} project={project} item={item} coordinator={coordinator} />
            : <Empty><EmptyDescription>Select a flow or role to see its content and history. This view is read-only; ask the Coordinator for changes.</EmptyDescription></Empty>}
    </ViewLayout>
}
