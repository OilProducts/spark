export type DefinitionLayer = 'bundled' | 'project' | 'project_copy'
export type DefinitionKind = 'flow' | 'role'
export type InboxEvent = { seq: number; id: string; at: string; kind: string; source: string; payload?: unknown }
/** A started role's record and its newest inbox events. */
export type LiveRole = {
    name: string
    title: string
    state: 'idle' | 'working' | 'stopped'
    conversation_id: string
    /** Present once the role's conversation exists. */
    conversation_handle?: string | null
    budget: { concurrent_runs: number }
    runs: { run_id: string; status: string }[]
    recent_events: InboxEvent[]
}
export type DefinitionItem = {
    kind: DefinitionKind
    name: string
    layer: DefinitionLayer
    path: string
    title?: string
    description?: string
    usage?: { flow: string; node: string }[]
    /** A role file that declares `standing: true`. */
    standing?: boolean
    /** The started role, or null. */
    live?: LiveRole | null
}
export type DefinitionList = { items: DefinitionItem[] }
export type DefinitionFile = { name: string; content: string }
export type RoleFileDetail = DefinitionItem & { files: DefinitionFile[]; bundled_diff: string | null }

// The inbox's terminal run statuses (TERMINAL_RUN_STATUSES in spark-workspace).
const TERMINAL = new Set(['completed', 'failed', 'validation_error', 'canceled'])

async function workspaceRequest<T>(route: string, project: string, params: Record<string, string> = {}, init?: RequestInit): Promise<T> {
    const query = new URLSearchParams({ project_path: project, ...params })
    const response = await fetch(`/workspace/api${route}?${query}`, init)
    const value = await response.json().catch(() => ({}))
    if (!response.ok) throw new Error(value.detail || `Request failed (${response.status})`)
    return value as T
}

const rolePath = (role: string, rest: string) => `/roles/${encodeURIComponent(role)}/${rest}`

/** The project's flows and roles by layer. */
export const fetchDefinitions = (project: string) => workspaceRequest<DefinitionList>('/definitions', project)

/** A role's winning file and, for a project copy, its diff from bundled. */
export const fetchRoleFile = (project: string, role: string) => workspaceRequest<RoleFileDetail>(rolePath(role, 'file'), project)

/** A role's memory files, or one file's text. */
export function fetchRoleMemory<T>(project: string, role: string, file?: string): Promise<T> {
    return workspaceRequest<T>(rolePath(role, 'memory'), project, file === undefined ? {} : { file })
}

/** Starts a standing role, creating its conversation if it has none. */
export const startRole = (project: string, role: string) =>
    workspaceRequest<{ conversation_id: string }>(rolePath(role, 'start'), project, {}, { method: 'POST' })

/** What a role's row and detail offer: open its conversation, start it, or nothing. */
export function roleAction(item: Pick<DefinitionItem, 'standing' | 'live'>): 'open' | 'start' | null {
    if (item.live?.conversation_handle) return 'open'
    return item.standing ? 'start' : null
}

/** Flow and role files are markdown under YAML frontmatter; show the frontmatter as a YAML block. */
export function renderableMarkdown(text: string): string {
    const { yaml, body } = splitFrontmatter(text)
    return yaml === null ? text : `\`\`\`yaml\n${yaml}\n\`\`\`\n\n${body}`
}

function splitFrontmatter(text: string): { yaml: string | null; body: string } {
    const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text)
    return match ? { yaml: match[1], body: text.slice(match[0].length) } : { yaml: null, body: text }
}

/** A file's top-level frontmatter keys and their values, and the markdown body. */
export function frontmatterFields(text: string): { fields: [string, string][]; body: string } {
    const { yaml, body } = splitFrontmatter(text)
    const fields: [string, string][] = []
    // ponytail: top-level `key: value` lines; a nested value (budget, execution) is kept as its indented text.
    for (const line of (yaml ?? '').split(/\r?\n/)) {
        const field = /^([\w-]+):\s*(.*)$/.exec(line)
        if (field) fields.push([field[1], field[2]])
        else if (fields.length && line.trim()) fields[fields.length - 1][1] += `${fields[fields.length - 1][1] ? '\n' : ''}${line.trim()}`
    }
    return { fields, body }
}

/** An item's origin when it is not bundled only: the project's own, or a project copy that overrides a bundled one. */
export function definitionBadges(item: Pick<DefinitionItem, 'layer'>): string[] {
    return item.layer === 'project' ? ['project'] : item.layer === 'project_copy' ? ['customized'] : []
}

/** A role's flow nodes, grouped by flow in first-seen order. */
export function usageByFlow(usage: { flow: string; node: string }[]): [string, string[]][] {
    const flows = new Map<string, string[]>()
    for (const use of usage) flows.set(use.flow, [...(flows.get(use.flow) ?? []), use.node])
    return [...flows]
}

/** "used in 3 steps across 2 flows". */
export function usageSummary(usage: { flow: string; node: string }[]): string {
    if (!usage.length) return 'not used in any flow'
    const flows = usageByFlow(usage).length
    return `used in ${usage.length} ${usage.length === 1 ? 'step' : 'steps'} across ${flows} ${flows === 1 ? 'flow' : 'flows'}`
}

/** "2/4 concurrent": current usage against the role's limit. */
export function budgetLine(role: LiveRole): string {
    const running = role.runs.filter((run) => !TERMINAL.has(run.status)).length
    return `${running}/${role.budget.concurrent_runs} concurrent`
}
