export type OrgLayer = 'bundled' | 'project' | 'project_copy'
export type OrgKind = 'flow' | 'role'
export type InboxEvent = { seq: number; id: string; at: string; kind: string; source: string; payload?: unknown }
export type StandingRole = {
    name: string
    title: string
    state: 'idle' | 'working' | 'stopped'
    conversation_id: string
    budget: { concurrent_runs: number; runs_per_day: number }
    recent_launches: number[]
    runs: { run_id: string; status: string }[]
    recent_events: InboxEvent[]
}
export type OrgItem = {
    kind: OrgKind
    name: string
    layer: OrgLayer
    path: string
    uncommitted: boolean
    usage?: { flow: string; node: string }[]
    standing?: StandingRole | null
}
export type OrgList = { org_path: string; bundled_path: string; org_exists: boolean; items: OrgItem[] }
export type OrgFile = { name: string; content: string; status: 'modified' | 'untracked' | null }
export type OrgItemDetail = OrgItem & { files: OrgFile[]; bundled_diff: string | null; bundled_changed: boolean | null }
export type OrgCommit = { commit: string; author: string; date: string; message: string; diff: string }

export const LAYER_LABELS: Record<OrgLayer, string> = { project: 'project', project_copy: 'project copy', bundled: 'bundled' }
export const LAYER_GROUPS: [OrgLayer, string][] = [['project', 'Project'], ['project_copy', 'Project copies'], ['bundled', 'Bundled']]
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
    const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text)
    return match ? `\`\`\`yaml\n${match[1]}\n\`\`\`\n\n${text.slice(match[0].length)}` : text
}

/** "2/4 concurrent · 3/20 today": current usage against the role's limits. */
export function budgetLine(role: StandingRole, nowSeconds = Date.now() / 1000): string {
    const running = role.runs.filter((run) => !TERMINAL.has(run.status)).length
    const today = role.recent_launches.filter((at) => at > nowSeconds - 86_400).length
    return `${running}/${role.budget.concurrent_runs} concurrent · ${today}/${role.budget.runs_per_day} today`
}
