export type OrgLayer = 'bundled' | 'project' | 'project_copy'
export type OrgKind = 'flow' | 'role'
export type InboxEvent = { seq: number; id: string; at: string; kind: string; source: string; payload?: unknown }
export type StandingRole = {
    name: string
    title: string
    state: 'idle' | 'working' | 'stopped'
    conversation_id: string
    budget: { concurrent_runs: number }
    runs: { run_id: string; status: string }[]
    recent_events: InboxEvent[]
}
export type OrgItem = {
    kind: OrgKind
    name: string
    layer: OrgLayer
    path: string
    title?: string
    description?: string
    usage?: { flow: string; node: string }[]
    standing?: StandingRole | null
}
export type OrgList = { org_path: string; bundled_path: string; org_exists: boolean; items: OrgItem[] }
// Definitions have no Git history of their own: items report no uncommitted state or bundled drift, files no status, and history is empty.
export type OrgFile = { name: string; content: string }
export type OrgItemDetail = OrgItem & { files: OrgFile[]; bundled_diff: string | null }

export const LAYER_LABELS: Record<OrgLayer, string> = { project: 'project', project_copy: 'project copy', bundled: 'bundled' }
// The inbox's terminal run statuses (TERMINAL_RUN_STATUSES in spark-workspace).
const TERMINAL = new Set(['completed', 'failed', 'validation_error', 'canceled'])

/** A read-only GET under /workspace/api/org for `project`. */
export async function orgRequest<T>(route: string, project: string, params: Record<string, string> = {}): Promise<T> {
    const query = new URLSearchParams({ project_path: project, ...params })
    const response = await fetch(`/workspace/api/org${route}?${query}`)
    const value = await response.json()
    if (!response.ok) throw new Error(value.detail || 'Org request failed')
    return value as T
}

/** The Coordinator draft for an item: what it is and where it lives, for the user to finish. */
export function askCoordinatorDraft(item: Pick<OrgItem, 'kind' | 'name' | 'layer' | 'path'>): string {
    return `About the ${item.kind} "${item.name}" (${LAYER_LABELS[item.layer]}, ${item.path}): `
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

/** An item's origin when it is not bundled only: the project's own, or a project copy of a bundled one. */
export function orgBadges(item: Pick<OrgItem, 'layer'>): string[] {
    return item.layer === 'project' ? ['customized'] : item.layer === 'project_copy' ? ['project copy'] : []
}

/** The roles' standing against the bundled defaults, from whether the project's definitions exist. */
export function orgStatus(list: OrgList): string {
    if (!list.org_exists) return 'No project definitions: all roles are bundled defaults.'
    const roles = list.items.filter((item) => item.kind === 'role')
    const customized = roles.filter((item) => item.layer === 'project').length
    const copies = roles.filter((item) => item.layer === 'project_copy').length
    return `${customized} customized ${customized === 1 ? 'role' : 'roles'} · ${copies} project ${copies === 1 ? 'copy' : 'copies'}`
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
export function budgetLine(role: StandingRole): string {
    const running = role.runs.filter((run) => !TERMINAL.has(run.status)).length
    return `${running}/${role.budget.concurrent_runs} concurrent`
}
