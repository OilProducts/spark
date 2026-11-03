import { mkdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, type Page } from '@playwright/test'

const testDir = path.dirname(fileURLToPath(import.meta.url))
const screenshotDir = path.resolve(testDir, '..', '..', 'artifacts', 'ui-smoke')
const smokeFixtureDir = path.join(testDir, 'flows')
/** The smoke flow as the JSON FlowDefinition the editor saves. */
export const smokeFixtureFlow: Record<string, unknown> = JSON.parse(readFileSync(path.join(smokeFixtureDir, 'editor-smoke-base.json'), 'utf-8'))
/** The same flow's files as saved: `editor-smoke-base/<file>`. */
export const smokeFixtureFile = (name: string) => readFileSync(path.join(smokeFixtureDir, 'editor-smoke-base', name), 'utf-8')
// Flows are per project: each smoke flow remembers the project it was saved in.
const smokeFlowRegistry = new WeakMap<Page, Map<string, string>>()

function getRegisteredSmokeFlows(page: Page): Map<string, string> {
  const existing = smokeFlowRegistry.get(page)
  if (existing) {
    return existing
  }
  const created = new Map<string, string>()
  smokeFlowRegistry.set(page, created)
  return created
}

export const ensureScreenshotDir = () => {
  mkdirSync(screenshotDir, { recursive: true })
}

export const screenshotPath = (name: string) => path.join(screenshotDir, name)

const buildSmokeProjectRecord = (projectPath: string) => {
  const displayName = path.basename(projectPath) || projectPath
  return {
    project_id: `smoke-${displayName}`,
    project_path: projectPath,
    display_name: displayName,
    created_at: '2026-03-11T09:00:00Z',
    last_opened_at: '2026-03-11T09:30:00Z',
    last_accessed_at: '2026-03-11T09:45:00Z',
    is_favorite: false,
    active_conversation_id: null,
  }
}

export async function stubProjectMetadata(page: Page, metadata?: { branch?: string; commit?: string }) {
  await page.route('**/workspace/api/projects/metadata**', async (route) => {
    const requestUrl = new URL(route.request().url())
    const directory = requestUrl.searchParams.get('directory') ?? ''
    const name = directory.split('/').filter(Boolean).pop() ?? directory
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        name,
        directory,
        branch: metadata?.branch ?? 'main',
        commit: metadata?.commit ?? 'smoke-test',
      }),
    })
  })
}

export async function stubProjectRegistration(page: Page, projectPath: string) {
  let registered = false
  const projectRecord = buildSmokeProjectRecord(projectPath)

  // Synthetic projects must not invoke host CLI model discovery during editor smoke tests.
  // Codex is reachable but lists nothing, so pickers still offer custom models.
  await page.route('**/workspace/api/projects/chat-models**', (route) => route.fulfill({
    json: { models: [], providers: { codex: { status: 'available', error: null } } },
  }))

  await page.route('**/workspace/api/projects', async (route) => {
    if (route.request().method() !== 'GET') {
      await route.continue()
      return
    }
    const requestUrl = new URL(route.request().url())
    if (requestUrl.pathname !== '/workspace/api/projects') {
      await route.continue()
      return
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(registered ? [projectRecord] : []),
    })
  })

  await page.route('**/workspace/api/projects/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(summarySnapshot([], new URL(route.request().url()).searchParams.get('project_path'))),
    })
  })

  await page.route('**/workspace/api/projects/browse**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        current_path: projectPath,
        parent_path: path.dirname(projectPath),
        entries: [],
      }),
    })
  })

  await page.route('**/workspace/api/projects/register', async (route) => {
    registered = true
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(projectRecord),
    })
  })

  await page.route('**/workspace/api/projects/state', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(projectRecord),
    })
  })
}

export const chatsProjectGroup = (page: Page, projectPath: string) =>
  page.locator(`[data-testid="chats-project-group"][data-project-path="${projectPath}"]`)

export async function registerProjectForSmokeTest(page: Page, projectPath: string) {
  await page.getByTestId('activity-chats').click()
  await page.getByTestId('add-project-button').click()
  await page.getByTestId('project-browser-select-button').click()
  // A new project joins the Chats panel, and the composer says chats go to it.
  await expect(chatsProjectGroup(page, projectPath)).toBeVisible()
  await expect(page.getByTestId('chat-composer-project')).toContainText(path.basename(projectPath) || projectPath)
}

/** Opens the editor's Run, which asks which project; picks `projectPath`. */
export async function runFlowInProject(page: Page, projectPath: string) {
  await page.getByTestId('editor-run-button').click()
  const picker = page.getByTestId('run-flow-project-picker')
  await expect(picker).toBeVisible()
  await picker.locator(`[data-testid="project-picker-item"][data-project-path="${projectPath}"]`).click()
}

export async function gotoWithRegisteredProject(page: Page, projectPath: string) {
  await stubProjectRegistration(page, projectPath)
  await page.goto('/')
  await registerProjectForSmokeTest(page, projectPath)
}

/** Saves a smoke flow into `projectPath`, the project whose editor lists it. */
export async function createFlowForSmokeTest(
  page: Page,
  flowPrefix: string,
  projectPath: string,
  flow: Record<string, unknown> = smokeFixtureFlow,
): Promise<string> {
  const flowName = `${flowPrefix}-${Date.now()}`
  const saveResponse = await page.request.post('/attractor/api/flows', {
    data: { name: flowName, flow, project_path: projectPath },
  })
  if (!saveResponse.ok()) {
    throw new Error(`Failed to persist smoke flow ${flowName}: HTTP ${saveResponse.status()}`)
  }
  getRegisteredSmokeFlows(page).set(flowName, projectPath)
  return flowName
}

export async function deleteFlowAfterSmoke(page: Page, flowName: string): Promise<void> {
  const projectPath = smokeFlowRegistry.get(page)?.get(flowName)
  if (!projectPath) {
    return
  }
  let response: Awaited<ReturnType<Page['request']['delete']>> | null = null
  try {
    response = await page.request.delete(`/attractor/api/flows/${encodeURIComponent(flowName)}?project_path=${encodeURIComponent(projectPath)}`, {
      timeout: 2_000,
    })
  } catch (error) {
    if (error instanceof Error && /timeout/i.test(error.message)) {
      return
    }
    throw error
  }
  if (!response) {
    return
  }
  if (!response.ok() && response.status() !== 404) {
    throw new Error(`Failed to delete smoke clone ${flowName}: HTTP ${response.status()}`)
  }
  const registered = smokeFlowRegistry.get(page)
  if (registered) {
    registered.delete(flowName)
    if (registered.size === 0) {
      smokeFlowRegistry.delete(page)
    }
  }
}

export async function cleanupSmokeFlowsForPage(page: Page): Promise<void> {
  const registered = smokeFlowRegistry.get(page)
  if (!registered || registered.size === 0) {
    return
  }
  const flowNames = [...registered.keys()]
  for (const flowName of flowNames) {
    try {
      await deleteFlowAfterSmoke(page, flowName)
    } catch {
      // Best-effort cleanup; afterEach should not fail due to stale temp flow deletion errors.
    }
  }
  smokeFlowRegistry.delete(page)
}

// Conversation wire fixtures (see src/test/conversationWire.ts, which e2e cannot import).
// Revisions, positions and orders are decimal strings, as on the wire.
type Json = Record<string, any>
export const SMOKE_GENERATION = 'generation-smoke'
const revisionOf = (fixture: Json) => String(fixture.revision ?? 1)
const maxRevision = (rows: Json[]) => String(rows.reduce((max, row) => (BigInt(row.revision) > max ? BigInt(row.revision) : max), 0n))

/** A conversation row from a summary or transcript-shaped fixture. */
export function conversationRow(fixture: Json, revision = revisionOf(fixture)): Json {
  const meta = { provider: fixture.provider ?? null, model: fixture.model ?? null, reasoning_effort: fixture.reasoning_effort ?? null }
  return {
    kind: 'conversation', conversation_id: fixture.conversation_id, id: fixture.conversation_id, revision, deleted: false,
    payload: {
      handle: fixture.conversation_handle ?? null, project_path: fixture.project_path, title: fixture.title ?? 'New thread',
      created_at: fixture.created_at ?? '', updated_at: fixture.updated_at ?? fixture.created_at ?? '',
      preview: fixture.last_message_preview ?? null, running: fixture.running ?? (fixture.turns ?? []).some((turn: Json) => turn.role === 'assistant' && ['pending', 'streaming'].includes(turn.status)), needs_attention: false,
      launched_run_ids: fixture.launched_run_ids ?? [],
      metadata: { conversation_id: fixture.conversation_id, project_path: fixture.project_path, chat_mode: fixture.chat_mode ?? 'chat', revision, ...meta },
      effective: { ...meta, llm_profile: null },
      ...(fixture.settings ? { settings: { ...fixture.settings, models: { ...fixture.settings.models, revision } } } : {}),
      ...(fixture.standing_role !== undefined ? { standing_role: fixture.standing_role } : {}),
    },
  }
}

/** The newest-window Page for a transcript-shaped fixture; a turn's position defaults to its timestamp in ms. */
export function conversationPage(fixture: Json): Json {
  const id = fixture.conversation_id
  const revision = revisionOf(fixture)
  const row = (kind: string, payload: Json, extra: Json) => ({ kind, conversation_id: id, id: payload.id, ...extra, revision, deleted: false, payload })
  const turns = (fixture.turns ?? []).map((turn: Json) => row('turn', turn, { position: String(turn.position ?? Date.parse(turn.timestamp)) }))
  const segments = (fixture.segments ?? []).map((segment: Json) => row('segment', segment, { turn_id: segment.turn_id, order: String(segment.order ?? 0) }))
  const artifacts = ['flow_run_requests', 'flow_launches', 'proposed_plans'].flatMap((collection) => (fixture[collection] ?? [])
    .map((artifact: Json, index: number) => row('artifact', artifact, { collection, position: String(index + 1) })))
  return {
    generation: SMOKE_GENERATION, watermark: revision, conversation_id: id,
    rows: [conversationRow(fixture, revision), ...turns, ...segments, ...artifacts],
    turn_ids: turns.map((turn: Json) => turn.id), pinned_turn_ids: [], pinned_artifact_ids: [], older_cursor: null,
  }
}

/** A thread-list response (`GET /projects/conversations`); each summary fixture becomes a conversation row. */
export function summarySnapshot(summaries: Json[], projectPath: string | null = null): Json {
  const rows = summaries.map((summary) => conversationRow(summary))
  return { generation: SMOKE_GENERATION, watermark: maxRevision(rows), project_path: projectPath, rows }
}

/** A revision batch of `rows`, starting just below its newest row unless `fromRevision` is given. */
export function conversationBatch(rows: Json[], fromRevision?: string): Json {
  const watermark = maxRevision(rows)
  return { generation: SMOKE_GENERATION, from_revision: fromRevision ?? String(BigInt(watermark) - 1n), watermark, rows }
}

/** One SSE frame for a conversation (`conversation.*`) or, without an id, the summary feed (`conversations.*`). */
export function liveFrame(type: string, payload: Json, scope: { conversationId?: string; projectPath?: string | null } = {}): string {
  const resource = scope.conversationId ? { kind: 'conversation', id: scope.conversationId } : { kind: 'conversations', id: null }
  return `data: ${JSON.stringify({ type, project_path: scope.projectPath ?? null, resource, cursor: null, payload })}\n\n`
}

/** The conversation row of a Page, SummarySnapshot or MutationAck from the server. */
export const conversationRowOf = (body: Json, id?: string): Json | undefined => (body.rows ?? body.changes?.rows ?? [])
  .find((row: Json) => row.kind === 'conversation' && (id === undefined || row.id === id))

/** The revision a settings PUT must expect for this conversation row. */
export const settingsRevisionOf = (row: Json): string => String(row.payload.settings?.models?.revision ?? row.payload.metadata.revision)

/** A Page's turn payloads in page order (oldest first); empty for an error body. */
export const pageTurns = (page: Json): Json[] => (page.turn_ids ?? [])
  .map((id: string) => page.rows.find((row: Json) => row.kind === 'turn' && row.id === id)?.payload)
