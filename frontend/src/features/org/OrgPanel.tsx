import { useEffect, useState, type ReactNode } from 'react'
import { MessageSquare, RefreshCw } from 'lucide-react'
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
    askCoordinatorDraft, budgetLine, frontmatterFields, orgBadges, orgRequest, orgStatus, renderableMarkdown, usageByFlow, usageSummary,
    type OrgCommit, type OrgItem, type OrgItemDetail, type OrgList, type StandingRole,
} from './orgModel'

/** Only what differs from the bundled default: customized, a drifted project copy, uncommitted. */
export function OrgBadges({ item }: { item: Pick<OrgItem, 'layer' | 'uncommitted' | 'bundled_changed'> }) {
    return <>{orgBadges(item).map((badge) => <Badge key={badge} data-testid="org-badge" variant={badge === 'uncommitted' ? 'destructive' : 'default'}>{badge}</Badge>)}</>
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

/** The flow nodes that use a role, each flow's nodes together. */
export function RoleUsage({ item }: { item: OrgItem }) {
    const flows = usageByFlow(item.usage ?? [])
    return flows.length
        ? <ul data-testid="org-role-usage" className="flex flex-col gap-0.5 text-sm">{flows.map(([flow, nodes]) => <li key={flow}>
            {flow} <span className="text-muted-foreground">· {nodes.join(', ')}</span>
        </li>)}</ul>
        : <p className="text-sm text-muted-foreground">No flow node uses this role.</p>
}

/** A role file: its frontmatter as key/value pairs above the rendered prompt. */
function RolePrompt({ content }: { content: string }) {
    const { fields, body } = frontmatterFields(content)
    return <div className="flex flex-col gap-3">
        {fields.length > 0 && <dl data-testid="org-frontmatter" className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            {fields.map(([key, value]) => <div key={key} className="contents">
                <dt className="text-muted-foreground">{key}</dt>
                <dd className="whitespace-pre-wrap">{value}</dd>
            </div>)}
        </dl>}
        <ProjectConversationMarkdown content={body} />
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
    return <div data-testid="org-live-status" className="flex flex-col gap-2 text-sm">
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

function RoleDetail({ project, item, coordinator }: { project: string; item: OrgItem; coordinator: StandingRole | null }) {
    const [detail, setDetail] = useState<OrgItemDetail | null>(null)
    const [commits, setCommits] = useState<OrgCommit[] | null>(null)
    const [error, setError] = useState('')
    useEffect(() => {
        orgRequest<OrgItemDetail>('/item', project, { kind: item.kind, name: item.name }).then(setDetail, (e: Error) => setError(e.message))
        orgRequest<{ commits: OrgCommit[] }>('/history', project, { path: item.path }).then((value) => setCommits(value.commits), (e: Error) => setError(e.message))
    }, [project, item.kind, item.name, item.path])
    return <section aria-label="Org role" className="flex h-full min-h-0 flex-col gap-4 overflow-y-auto p-4">
        <header className="flex flex-wrap items-center gap-2">
            <h2 className="text-base font-semibold">{item.title ?? item.name}</h2>
            {item.title && item.title !== item.name && <span className="text-xs text-muted-foreground">{item.name}</span>}
            <OrgBadges item={item} />
            <span className="flex-1" />
            {item.name !== 'coordinator' && <Button type="button" size="sm" variant="outline" disabled={!coordinator}
                onClick={() => coordinator && openChat(project, coordinator.conversation_id, askCoordinatorDraft(item))}>
                <MessageSquare aria-hidden="true" />Ask the Coordinator
            </Button>}
        </header>
        {item.description && <p className="text-sm">{item.description}</p>}
        <p className="text-xs text-muted-foreground">{item.path}</p>
        {error && <InlineError dense>{error}</InlineError>}
        {item.standing && <Section title="Live status"><StandingDetails project={project} role={item.standing} /></Section>}
        <Section title="Used in"><RoleUsage item={item} /></Section>
        <Section title="Prompt">
            {detail ? detail.files.map((file) => <div key={file.name} data-testid="org-file" className="rounded-md border p-3">
                {file.status && <div className="mb-2"><Badge variant="destructive">{file.status}</Badge></div>}
                <RolePrompt content={file.content} />
            </div>) : !error && <p className="text-xs text-muted-foreground">Loading…</p>}
        </Section>
        {item.layer === 'project_copy' && <Section title="Diff from bundled">
            <p data-testid="org-bundled-changed" className="text-sm">{item.bundled_changed == null
                ? 'The copy is not committed yet, so its bundled origin is unknown.'
                : item.bundled_changed ? 'Bundled has changed since this copy was made.' : 'Bundled is unchanged since this copy was made.'}</p>
            <p className="text-xs text-muted-foreground">Diff from today's bundled version to the copy:</p>
            {detail ? <DiffBlock diff={detail.bundled_diff ?? ''} /> : !error && <p className="text-xs text-muted-foreground">Loading…</p>}
        </Section>}
        <details aria-label="History" className="flex flex-col gap-2">
            <summary className="cursor-pointer text-sm font-semibold">History</summary>
            <div className="mt-2"><OrgHistory commits={commits} /></div>
        </details>
    </section>
}

/** A read-only view of a project's roles, standing ones first; changes go through the Coordinator. */
export function OrgPanel() {
    const registry = useStore((state) => state.projectRegistry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const [project, setProject] = useState(() => activeProjectPath ?? defaultProjectChoice(registry))
    const [list, setList] = useState<OrgList | null>(null)
    const [selected, setSelected] = useState<string | null>(null)
    const [error, setError] = useState('')
    const [refreshes, setRefreshes] = useState(0)
    useEffect(() => {
        if (!project) return
        let current = true
        setError('')
        orgRequest<OrgList>('', project).then((value) => current && setList(value), (e: Error) => current && setError(e.message))
        return () => { current = false }
    }, [project, refreshes])
    const roles = (list?.items ?? []).filter((entry) => entry.kind === 'role')
    const item = roles.find((entry) => entry.name === selected) ?? null
    const coordinator = roles.find((entry) => entry.name === 'coordinator')?.standing ?? null
    const groups: [string, OrgItem[]][] = [['Standing', roles.filter((entry) => entry.standing)], ['Roles', roles.filter((entry) => !entry.standing)]]
    const panel = <div className="flex flex-col gap-3">
        {list && <p data-testid="org-status" className="px-3 text-xs text-muted-foreground" title={list.org_path}>{orgStatus(list)}</p>}
        {error && <InlineError>{error}</InlineError>}
        {groups.map(([label, grouped]) => grouped.length > 0 && <section key={label} aria-label={label} className="flex flex-col gap-0.5">
            <ListGroupLabel><h3>{label}</h3></ListGroupLabel>
            <ul className="flex flex-col gap-0.5">{grouped.map((entry) => <li key={entry.name} className="flex items-start gap-1">
                <SidePanelRow title={entry.title ?? entry.name} data-testid="org-item" data-name={entry.name}
                    selected={item === entry}
                    meta={<>
                        {entry.standing
                            ? <><span className="block truncate">{entry.standing.state} · {budgetLine(entry.standing)}</span>
                                <span className="block truncate">{entry.standing.recent_events.length
                                    ? `Last wake ${entry.standing.recent_events[0].kind} · ${entry.standing.recent_events[0].at}`
                                    : 'No wakes yet'}</span></>
                            : <>{entry.description && <span className="block truncate">{entry.description}</span>}
                                <span className="block truncate">{usageSummary(entry.usage ?? [])}</span></>}
                        {/* Badges sit below the text so the title and meta keep the row's width. */}
                        {orgBadges(entry).length > 0 && <span className="flex flex-wrap gap-1 pt-1"><OrgBadges item={entry} /></span>}
                    </>}
                    onClick={() => setSelected(entry.name)} />
                {entry.standing && <Button type="button" variant="ghost" size="icon-sm" className="mt-1.5" aria-label={`Open conversation with ${entry.title ?? entry.name}`}
                    onClick={() => entry.standing && openChat(project!, entry.standing.conversation_id)}>
                    <MessageSquare aria-hidden="true" className="size-4" />
                </Button>}
            </li>)}</ul>
        </section>)}
        {!project && <Empty className="text-xs"><EmptyDescription>Choose a project</EmptyDescription></Empty>}
        {project && !list && !error && <p className="px-3 py-2 text-sm text-muted-foreground">Loading…</p>}
    </div>
    return <ViewLayout view="org" title="Org" panel={panel} actions={<>
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh" onClick={() => setRefreshes((value) => value + 1)}><RefreshCw aria-hidden="true" className="size-4" /></Button>
        <ProjectPicker heading="Show the org of" testId="org-project-picker" defaultProjectPath={project} onPick={(path) => { setProject(path); setSelected(null); setList(null) }}>
            <Button type="button" variant="outline" size="sm" className="max-w-36" title={project ?? undefined}><span className="truncate">{project ? projectLabel(registry, project) : 'Project'}</span></Button>
        </ProjectPicker>
    </>}>
        {project && item
            ? <RoleDetail key={`${project}:${item.name}:${refreshes}`} project={project} item={item} coordinator={coordinator} />
            : <div data-testid="org-empty-state" className="flex h-full items-center justify-center p-6">
                <Empty className="max-w-md items-center text-center"><EmptyDescription>Select a role to see its prompt, where it is used and its history. This view is read-only; ask the Coordinator for changes.</EmptyDescription></Empty>
            </div>}
    </ViewLayout>
}
