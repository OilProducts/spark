import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronDown, MessageSquare, Network, Play, RefreshCw } from 'lucide-react'
import { useStore } from '@/store'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/app/inline-error'
import { ListGroupLabel, SidePanelRow } from '@/components/app/side-panel-row'
import { SectionHeading } from '@/components/app/section-heading'
import { SheetEmpty, ViewLayout } from '@/components/app/view-layout'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { defaultProjectChoice, projectLabel } from '@/features/projects/model/projectChoices'
import { openChat } from '@/app/useAttention'
import { cn } from '@/lib/utils'
import {
    budgetLine, definitionBadges, fetchDefinitions, fetchRoleFile, fetchRoleMemory, frontmatterFields, renderableMarkdown, roleAction, startRole,
    usageByFlow, usageSummary, type DefinitionItem, type DefinitionList, type LiveRole, type RoleFileDetail,
} from './rolesModel'

/** Only what differs from bundled only: customized or a project copy. */
export function DefinitionBadges({ item }: { item: Pick<DefinitionItem, 'layer'> }) {
    return <>{definitionBadges(item).map((badge) => <Badge key={badge} data-testid="definition-badge" variant="default">{badge}</Badge>)}</>
}

/** A unified diff with added and removed lines marked. */
export function DiffBlock({ diff }: { diff: string }) {
    if (!diff.trim()) return <p className="text-xs text-muted-foreground">No differences.</p>
    return <pre data-testid="roles-diff" className="max-h-96 overflow-auto rounded-md border bg-muted/40 p-2 text-xs leading-5">
        {diff.split('\n').map((line, index) => <div key={index} className={cn(
            line.startsWith('+') && !line.startsWith('+++') && 'text-success',
            line.startsWith('-') && !line.startsWith('---') && 'text-destructive',
            line.startsWith('@@') && 'text-muted-foreground',
        )}>{line || ' '}</div>)}
    </pre>
}

function Section({ title, children }: { title: string; children: ReactNode }) {
    return <section aria-label={title} className="flex flex-col gap-2">
        <h2 className="text-base font-semibold">{title}</h2>
        {children}
    </section>
}

/** The flow nodes that use a role, each flow's nodes together. */
export function RoleUsage({ item }: { item: DefinitionItem }) {
    const flows = usageByFlow(item.usage ?? [])
    return flows.length
        ? <ul data-testid="role-usage" className="flex flex-col gap-0.5 text-sm">{flows.map(([flow, nodes]) => <li key={flow}>
            {flow} <span className="text-muted-foreground">· {nodes.join(', ')}</span>
        </li>)}</ul>
        : <p className="text-sm text-muted-foreground">No flow node uses this role.</p>
}

/** A role file: its frontmatter as key/value pairs above the rendered prompt. */
function RolePrompt({ content }: { content: string }) {
    const { fields, body } = frontmatterFields(content)
    return <div className="flex flex-col gap-3">
        {fields.length > 0 && <dl data-testid="role-frontmatter" className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
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
        fetchRoleMemory<{ files: string[] }>(project, role).then((value) => setFiles(value.files), (e: Error) => setError(e.message))
    }, [project, role])
    const read = (file: string) => fetchRoleMemory<{ content: string }>(project, role, file)
        .then((value) => setOpen({ file, content: value.content }), (e: Error) => setError(e.message))
    if (error) return <InlineError dense>{error}</InlineError>
    if (!files) return <p className="text-xs text-muted-foreground">Loading…</p>
    if (!files.length) return <p className="text-xs text-muted-foreground">No memory files.</p>
    return <div className="flex flex-col gap-2">
        <ul className="flex flex-wrap gap-1">{files.map((file) => <li key={file}>
            <Button type="button" size="xs" variant={open?.file === file ? 'outline' : 'ghost'} aria-pressed={open?.file === file} onClick={() => void read(file)}>{file}</Button>
        </li>)}</ul>
        {open && <div data-testid="role-memory-file" className="rounded-md border p-3"><ProjectConversationMarkdown content={renderableMarkdown(open.content)} /></div>}
    </div>
}

function LiveDetails({ project, role }: { project: string; role: LiveRole }) {
    return <div data-testid="role-live-status" className="flex flex-col gap-2 text-sm">
        <p><span className="text-muted-foreground">State</span> {role.state} · <span className="text-muted-foreground">Budget</span> {budgetLine(role)}</p>
        <SectionHeading as="h3">Recent wakes</SectionHeading>
        {role.recent_events.length
            ? <ul className="flex flex-col gap-0.5 text-xs">{role.recent_events.map((event) => <li key={event.seq}>{event.kind} · {event.source} · <span className="text-muted-foreground">{event.at}</span></li>)}</ul>
            : <p className="text-xs text-muted-foreground">No wakes yet.</p>}
        <SectionHeading as="h3">Memory</SectionHeading>
        <MemoryBrowser project={project} role={role.name} />
    </div>
}

type RoleControl = { project: string; item: DefinitionItem; pending: boolean; onStart: () => void }

/** Start or Open conversation, by `roleAction`; the row shows it as an icon. */
function RoleButton({ project, item, pending, onStart, compact }: RoleControl & { compact?: boolean }) {
    const action = roleAction(item)
    if (!action) return null
    const name = item.title ?? item.name
    const [label, Icon, onClick] = action === 'open'
        ? ['Open conversation', MessageSquare, () => openChat(project, item.live!.conversation_id)] as const
        : ['Start', Play, onStart] as const
    return compact
        ? <Button type="button" variant="ghost" size="icon-sm" className="mt-1.5" disabled={pending} aria-label={`${label} ${action === 'open' ? 'with ' : ''}${name}`} onClick={onClick}>
            <Icon aria-hidden="true" className="size-4" />
        </Button>
        : <Button type="button" size="sm" variant="outline" disabled={pending} onClick={onClick}><Icon aria-hidden="true" />{pending ? 'Starting…' : label}</Button>
}

function RoleDetail({ project, item, pending, onStart, startError }: RoleControl & { startError?: string }) {
    const [detail, setDetail] = useState<RoleFileDetail | null>(null)
    const [error, setError] = useState('')
    useEffect(() => {
        fetchRoleFile(project, item.name).then(setDetail, (e: Error) => setError(e.message))
    }, [project, item.name])
    return <section aria-label="Role" className="flex h-full min-h-0 flex-col gap-4 overflow-y-auto p-6">
        <header className="flex flex-wrap items-center gap-2">
            <h1 className="text-xl font-semibold tracking-tight">{item.title ?? item.name}</h1>
            {item.title && item.title !== item.name && <span className="text-xs text-muted-foreground">{item.name}</span>}
            <DefinitionBadges item={item} />
            <span className="flex-1" />
            <RoleButton project={project} item={item} pending={pending} onStart={onStart} />
        </header>
        {startError && <InlineError dense>{startError}</InlineError>}
        {item.description && <p className="text-sm">{item.description}</p>}
        <p className="text-xs text-muted-foreground">{item.path}</p>
        {error && <InlineError dense>{error}</InlineError>}
        {item.live && <Section title="Live status"><LiveDetails project={project} role={item.live} /></Section>}
        <Section title="Used in"><RoleUsage item={item} /></Section>
        <Section title="Prompt">
            {detail ? detail.files.map((file) => <div key={file.name} data-testid="role-file" className="rounded-md border p-3">
                <RolePrompt content={file.content} />
            </div>) : !error && <p className="text-xs text-muted-foreground">Loading…</p>}
        </Section>
        {item.layer === 'project_copy' && <Section title="Diff from bundled">
            <p className="text-xs text-muted-foreground">Diff from today's bundled version to the copy:</p>
            {detail ? <DiffBlock diff={detail.bundled_diff ?? ''} /> : !error && <p className="text-xs text-muted-foreground">Loading…</p>}
        </Section>}
    </section>
}

/** A project's resolved roles, standing ones first, where a standing role is started or opened. */
export function RolesPanel() {
    const registry = useStore((state) => state.projectRegistry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const [project, setProject] = useState(() => activeProjectPath ?? defaultProjectChoice(registry))
    const [list, setList] = useState<DefinitionList | null>(null)
    const [selected, setSelected] = useState<string | null>(null)
    const [error, setError] = useState('')
    const [refreshes, setRefreshes] = useState(0)
    // Starts in flight and their failures, by project and role, so a start outlives a project switch.
    const [starting, setStarting] = useState<ReadonlySet<string>>(new Set())
    const [startErrors, setStartErrors] = useState<Record<string, string>>({})
    const projectRef = useRef(project)
    useEffect(() => {
        projectRef.current = project
        if (!project) return
        let current = true
        setError('')
        fetchDefinitions(project).then((value) => current && setList(value), (e: Error) => current && setError(e.message))
        return () => { current = false }
    }, [project, refreshes])
    const start = (target: string, role: string) => {
        const key = `${target}\0${role}`
        setStarting((keys) => new Set(keys).add(key))
        setStartErrors((errors) => { const next = { ...errors }; delete next[key]; return next })
        startRole(target, role).then((started) => {
            openChat(target, started.conversation_id)
            if (projectRef.current === target) setRefreshes((value) => value + 1)
        }, (e: Error) => setStartErrors((errors) => ({ ...errors, [key]: e.message })))
            .finally(() => setStarting((keys) => { const next = new Set(keys); next.delete(key); return next }))
    }
    const control = (entry: DefinitionItem): RoleControl => ({
        project: project!, item: entry, pending: starting.has(`${project}\0${entry.name}`), onStart: () => start(project!, entry.name),
    })
    const startError = (entry: DefinitionItem) => startErrors[`${project}\0${entry.name}`]
    const roles = (list?.items ?? []).filter((entry) => entry.kind === 'role')
    const item = roles.find((entry) => entry.name === selected) ?? null
    const groups: [string, DefinitionItem[]][] = [['Standing', roles.filter((entry) => entry.standing)], ['Roles', roles.filter((entry) => !entry.standing)]]
    const panel = <div className="flex flex-col gap-3">
        {error && <InlineError>{error}</InlineError>}
        {groups.map(([label, grouped]) => grouped.length > 0 && <section key={label} aria-label={label} className="flex flex-col gap-0.5">
            <ListGroupLabel><h3>{label}</h3></ListGroupLabel>
            <ul className="flex flex-col gap-0.5">{grouped.map((entry) => <li key={entry.name} className="flex flex-col">
                <div className="flex items-start gap-1">
                    <SidePanelRow title={entry.title ?? entry.name} data-testid="roles-item" data-name={entry.name}
                        selected={item === entry}
                        meta={<>
                            {entry.live
                                ? <><span className="block truncate">{entry.live.state} · {budgetLine(entry.live)}</span>
                                    <span className="block truncate">{entry.live.recent_events.length
                                        ? `Last wake ${entry.live.recent_events[0].kind} · ${entry.live.recent_events[0].at}`
                                        : 'No wakes yet'}</span></>
                                : <>{entry.description && <span className="block truncate">{entry.description}</span>}
                                    <span className="block truncate">{usageSummary(entry.usage ?? [])}</span></>}
                            {/* Badges sit below the text so the title and meta keep the row's width. */}
                            {definitionBadges(entry).length > 0 && <span className="flex flex-wrap gap-1 pt-1"><DefinitionBadges item={entry} /></span>}
                        </>}
                        onClick={() => setSelected(entry.name)} />
                    <RoleButton {...control(entry)} compact />
                </div>
                {startError(entry) && <InlineError dense>{startError(entry)}</InlineError>}
            </li>)}</ul>
        </section>)}
        {!project && <p className="px-3 py-1 text-sm text-muted-foreground">Choose a project</p>}
        {project && !list && !error && <p className="px-3 py-1 text-sm text-muted-foreground">Loading…</p>}
    </div>
    return <ViewLayout view="roles" title="Roles" panel={panel} actions={<>
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh roles" title="Refresh roles" onClick={() => setRefreshes((value) => value + 1)}><RefreshCw aria-hidden="true" /></Button>
        <ProjectPicker heading="Show the roles of" testId="roles-project-picker" defaultProjectPath={project} onPick={(path) => { setProject(path); setSelected(null); setList(null) }}>
            <Button type="button" variant="outline" size="sm" className="max-w-36" title={project ?? undefined}><span className="truncate">{project ? projectLabel(registry, project) : 'Project'}</span><ChevronDown aria-hidden="true" className="text-muted-foreground" /></Button>
        </ProjectPicker>
    </>}>
        {project && item
            ? <RoleDetail key={`${project}:${item.name}:${refreshes}`} {...control(item)} startError={startError(item)} />
            : <SheetEmpty data-testid="roles-empty-state" icon={Network} title="No role selected">
                Select a role to see its prompt and where it is used. Start a standing role to open its conversation.
            </SheetEmpty>}
    </ViewLayout>
}
