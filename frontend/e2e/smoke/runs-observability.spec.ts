import { expect, test, type Page } from '@playwright/test'
import { ensureScreenshotDir, gotoWithRegisteredProject, screenshotPath, stubProjectMetadata } from '../fixtures/smoke-helpers'

type SmokeRunRecord = {
  run_id: string
  flow_name: string
  status: string
  outcome: 'success' | 'failure' | null
  working_directory: string
  project_path: string
  git_branch: string | null
  git_commit: string | null
  model: string
  started_at: string
  ended_at: string | null
  last_error: string
  token_usage: number
  token_usage_breakdown?: {
    input_tokens: number
    cached_input_tokens: number
    output_tokens: number
    total_tokens: number
    by_model: Record<string, {
      input_tokens: number
      cached_input_tokens: number
      output_tokens: number
      total_tokens: number
    }>
  } | null
  estimated_model_cost?: {
    currency: string
    amount: number
    status: 'estimated' | 'partial_unpriced' | 'unpriced'
    unpriced_models: string[]
    by_model?: Record<string, {
      currency: string
      amount: number | null
      status: 'estimated' | 'unpriced'
    }>
  } | null
  current_node?: string | null
  title?: string | null
  parent_run_id?: string | null
  root_run_id?: string | null
}

type SmokeJournalEntry = {
  id: string
  sequence: number
  emitted_at: string
  kind: string
  raw_type: string
  severity: 'info' | 'warning' | 'error'
  summary: string
  node_id?: string | null
  stage_index?: number | null
  source_scope?: 'root' | 'child' | null
  source_parent_node_id?: string | null
  source_flow_name?: string | null
  question_id?: string | null
  payload: Record<string, unknown>
}

test.beforeAll(() => {
  ensureScreenshotDir()
})

test.beforeEach(async ({ page }) => {
  await stubProjectMetadata(page)
})

function buildSmokeRun(projectPath: string, overrides: Partial<SmokeRunRecord> = {}): SmokeRunRecord {
  return {
    run_id: overrides.run_id ?? `run-${Date.now()}`,
    flow_name: overrides.flow_name ?? 'SmokeFlow',
    status: overrides.status ?? 'completed',
    outcome: overrides.outcome ?? 'success',
    working_directory: overrides.working_directory ?? `${projectPath}/workspace`,
    project_path: overrides.project_path ?? projectPath,
    git_branch: overrides.git_branch ?? 'main',
    git_commit: overrides.git_commit ?? 'abc1234',
    model: overrides.model ?? 'gpt-5',
    started_at: overrides.started_at ?? '2026-03-03T12:00:00Z',
    ended_at: overrides.ended_at ?? '2026-03-03T12:02:00Z',
    last_error: overrides.last_error ?? '',
    token_usage: overrides.token_usage ?? 42,
    token_usage_breakdown: overrides.token_usage_breakdown ?? {
      input_tokens: 28,
      cached_input_tokens: 6,
      output_tokens: 14,
      total_tokens: overrides.token_usage ?? 42,
      by_model: {
        'gpt-5.4': {
          input_tokens: 28,
          cached_input_tokens: 6,
          output_tokens: 14,
          total_tokens: overrides.token_usage ?? 42,
        },
      },
    },
    estimated_model_cost: overrides.estimated_model_cost ?? {
      currency: 'USD',
      amount: 0.000257,
      status: 'estimated',
      unpriced_models: [],
      by_model: {
        'gpt-5.4': {
          currency: 'USD',
          amount: 0.000257,
          status: 'estimated',
        },
      },
    },
    current_node: overrides.current_node ?? null,
    title: overrides.title ?? null,
    parent_run_id: overrides.parent_run_id ?? null,
    root_run_id: overrides.root_run_id ?? null,
  }
}

function buildSmokeJournalEntry(sequence: number, overrides: Partial<SmokeJournalEntry>): SmokeJournalEntry {
  return {
    id: overrides.id ?? `journal-${sequence}`,
    sequence,
    emitted_at: overrides.emitted_at ?? `2026-03-03T12:00:${String(sequence).padStart(2, '0')}Z`,
    kind: overrides.kind ?? 'log',
    raw_type: overrides.raw_type ?? 'log',
    severity: overrides.severity ?? 'info',
    summary: overrides.summary ?? `Journal entry ${sequence}`,
    node_id: overrides.node_id ?? null,
    stage_index: overrides.stage_index ?? null,
    source_scope: overrides.source_scope ?? 'root',
    source_parent_node_id: overrides.source_parent_node_id ?? null,
    source_flow_name: overrides.source_flow_name ?? null,
    question_id: overrides.question_id ?? null,
    payload: overrides.payload ?? {},
  }
}

async function stubRunSummary(page: Page, run: SmokeRunRecord, listedRuns: SmokeRunRecord[] = [run]) {
  await page.route('**/attractor/runs**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        runs: listedRuns,
      }),
    })
  })

  await page.route(`**/attractor/pipelines/${run.run_id}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        ...run,
        completed_nodes: run.current_node ? [run.current_node] : [],
        progress: {
          current_node: run.current_node ?? null,
          completed_count: run.current_node ? 1 : 0,
        },
      }),
    })
  })
}

async function installMockEventSource(page: Page) {
  await page.addInitScript(() => {
    class MockEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      static readonly instances: MockEventSource[] = []

      readonly url: string
      readyState = MockEventSource.CONNECTING
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
        MockEventSource.instances.push(this)
        window.setTimeout(() => {
          if (this.readyState === MockEventSource.CLOSED) {
            return
          }
          this.readyState = MockEventSource.OPEN
          this.onopen?.(new Event('open'))
        }, 0)
      }

      emit(payload: unknown) {
        if (this.readyState === MockEventSource.CLOSED) {
          return
        }
        this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(payload) }))
      }

      close() {
        this.readyState = MockEventSource.CLOSED
      }
    }

    ;(globalThis as typeof globalThis & {
      __runEventSourceController?: {
        latestUrl(pattern: string): string | null
        emitLatest(pattern: string, payload: unknown): void
      }
    }).__runEventSourceController = {
      latestUrl(pattern: string) {
        const match = [...MockEventSource.instances]
          .reverse()
          .find((eventSource) => eventSource.url.includes(pattern))
        return match?.url ?? null
      },
      emitLatest(pattern: string, payload: unknown) {
        const match = [...MockEventSource.instances]
          .reverse()
          .find((eventSource) => eventSource.url.includes(pattern))
        if (!match) {
          throw new Error(`No mock EventSource found for pattern: ${pattern}`)
        }
        match.emit(payload)
      },
    }

    Object.defineProperty(globalThis, 'EventSource', {
      configurable: true,
      writable: true,
      value: MockEventSource,
    })
  })
}

async function stubJson(page: Page, pattern: string, body: unknown) {
  await page.route(pattern, async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })
  })
}

function stageJournal(stages: Array<[string, number, 'success' | 'fail' | 'running']>): SmokeJournalEntry[] {
  return stages.flatMap(([node, index, outcome]) => [
    buildSmokeJournalEntry(index * 2 + 1, {
      kind: 'stage', raw_type: 'StageStarted', summary: `Stage ${node} started`, node_id: node, stage_index: index,
      emitted_at: `2026-03-03T12:0${index}:00Z`, payload: { node_id: node, index },
    }),
    ...(outcome === 'running' ? [] : [buildSmokeJournalEntry(index * 2 + 2, {
      kind: 'stage',
      raw_type: outcome === 'fail' ? 'StageFailed' : 'StageCompleted',
      summary: `Stage ${node} ${outcome === 'fail' ? 'failed' : 'completed'}`,
      node_id: node,
      stage_index: index,
      emitted_at: `2026-03-03T12:0${index}:42Z`,
      payload: outcome === 'fail' ? { node_id: node, index, error: 'stage_failed' } : { node_id: node, index, outcome },
    })]),
  ]).reverse()
}

async function openRunsForSmokeTest(page: Page, projectPath: string) {
  await gotoWithRegisteredProject(page, projectPath)
  await page.getByTestId('nav-mode-runs').click()
  await expect(page.getByTestId('run-history-row').first()).toBeVisible()
  await page.getByTestId('run-history-row').first().click()
  await expect(page.getByTestId('run-summary-panel')).toBeVisible()
}

test('a completed run opens on its status item with its result, source, what did not pass, and outputs', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-summary-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-summary-${Date.now()}`,
    flow_name: 'software-development/smoke-flow.yaml',
    title: 'Tighten the smoke loop',
    git_branch: 'feature/traceability',
    git_commit: 'fedcba9876543210',
  })

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        ...run,
        executions: [
          { run_id: run.run_id, node_id: 'review', stage_index: 0, attempt: 0, status: { outcome: 'fail', failure_reason: 'Missing a regression test.' } },
          { run_id: run.run_id, node_id: 'commit', stage_index: 1, attempt: 0, status: { outcome: 'success' } },
        ],
        child_runs: [],
      }),
    })
  })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/journal**`, {
    pipeline_id: run.run_id,
    entries: stageJournal([['review', 0, 'fail'], ['commit', 1, 'success']]),
    oldest_sequence: 1,
    newest_sequence: 4,
    has_older: false,
  })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/graph-preview**`, {
    status: 'ok',
    flow: { title: 'Smoke Flow', nodes: { review: { kind: 'agent_task', label: 'Review' }, commit: { kind: 'tool', label: 'Commit Findings' } } },
    graph: { nodes: [], edges: [] },
    diagnostics: [],
    errors: [],
  })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/executions/*/*/transcript`, { records: [] })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/result`, {
    run_id: run.run_id,
    status: 'completed',
    state: 'ready',
    source_node_id: 'commit',
    source_artifact_path: 'logs/commit/response.md',
    display_mode: 'raw',
    body_markdown: 'Committed **the fix**.',
    summary_enabled: false,
  })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/artifacts`, {
    pipeline_id: run.run_id,
    artifacts: [
      { path: 'result/result.md', size_bytes: 20, media_type: 'text/markdown', viewable: true },
      { path: 'artifacts/flow/flow-source.yaml', size_bytes: 20, media_type: 'text/yaml', viewable: true },
    ],
  })
  await openRunsForSmokeTest(page, projectPath)

  // The header is a title under its flow and one line of facts.
  await expect(page.getByTestId('run-header-flow')).toHaveText('Smoke Flow')
  await expect(page.getByTestId('run-header-title')).toHaveText('Tighten the smoke loop')
  await expect(page.getByTestId('run-header-facts')).toContainText('Completed')
  await expect(page.getByTestId('run-header-facts')).toContainText('42 tokens · fedcba9')
  await expect(page.getByTestId('run-summary-retry-button')).toHaveCount(0)
  await expect(page.getByTestId('run-summary-cancel-button')).toHaveCount(0)
  await expect(page.getByRole('tablist')).toHaveCount(0)

  const status = page.getByTestId('run-status-item')
  await expect(page.getByTestId('run-visit-item-status')).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByTestId('run-result-body')).toContainText('Committed the fix.')
  await expect(page.getByTestId('run-status-result-source')).toHaveText(' · from Commit Findings, raw output')
  await expect(page.getByTestId('run-status-did-not-pass')).toContainText('Review: Missing a regression test.')
  await expect(page.getByTestId('run-output-commit')).toHaveText('fedcba9 on feature/traceability')
  await expect(page.getByTestId('run-output-file')).toHaveText(['result.md', 'flow snapshot'])

  // Run facts stay collapsed until asked for.
  await expect(page.getByTestId('run-fact-cost')).toBeHidden()
  await status.getByText('Run facts').click()
  await expect(page.getByTestId('run-fact-cost')).toContainText('$0.000257')
  await expect(page.getByTestId('run-fact-model-usage')).toContainText('gpt-5.4')
  await page.screenshot({ path: screenshotPath('08b-runs-panel-populated-summary.png'), fullPage: true })

  await status.getByTestId('run-visit-link').click()
  await expect(page.getByTestId('run-visit-view-title')).toHaveText('Review')
})

test('run journal inspector hydrates durable history, pages older entries, and applies live tail updates with pinned questions', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-journal-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-journal-${Date.now()}`,
    flow_name: 'JournalFlow',
    status: 'running',
    outcome: null,
    ended_at: null,
    current_node: 'approve_release',
  })
  const journalRequestUrls: string[] = []
  const latestEntries = [
    buildSmokeJournalEntry(5, {
      kind: 'interview',
      raw_type: 'human_gate',
      summary: 'Human gate pending: Approve production deploy?',
      node_id: 'approve_release',
      stage_index: 3,
      question_id: 'gate-approve',
      payload: {
        node_id: 'approve_release',
        prompt: 'Approve production deploy?',
        question_id: 'gate-approve',
        question_type: 'YES_NO',
        options: [
          {
            label: 'Approve',
            value: 'YES',
            key: 'Y',
            description: 'Ship the release',
          },
          {
            label: 'Hold',
            value: 'NO',
            key: 'N',
            description: 'Keep the gate closed',
          },
        ],
      },
    }),
    buildSmokeJournalEntry(4, {
      kind: 'log',
      raw_type: 'log',
      summary: 'Deploy package uploaded to staging.',
      payload: {
        msg: 'Deploy package uploaded to staging.',
      },
    }),
  ]
  const olderEntries = [
    buildSmokeJournalEntry(3, {
      kind: 'stage',
      raw_type: 'StageCompleted',
      summary: 'Stage build completed',
      node_id: 'build',
      stage_index: 2,
      payload: {
        node_id: 'build',
        outcome: 'success',
      },
    }),
    buildSmokeJournalEntry(2, {
      kind: 'stage',
      raw_type: 'StageStarted',
      summary: 'Stage build started',
      node_id: 'build',
      stage_index: 2,
      payload: {
        node_id: 'build',
      },
    }),
  ]

  await installMockEventSource(page)
  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}/journal**`, async (route) => {
    const requestUrl = new URL(route.request().url())
    journalRequestUrls.push(requestUrl.toString())
    const beforeSequence = requestUrl.searchParams.get('before_sequence')
    const isOlderPage = beforeSequence === '4'
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        entries: isOlderPage ? olderEntries : latestEntries,
        oldest_sequence: isOlderPage ? 2 : 4,
        newest_sequence: isOlderPage ? 3 : 5,
        has_older: !isOlderPage,
      }),
    })
  })

  const answers: unknown[] = []
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/questions`, {
    pipeline_id: run.run_id,
    questions: [{
      question_id: 'gate-approve',
      node_id: 'approve_release',
      prompt: 'Approve production deploy?',
      question_type: 'YES_NO',
      options: [{ label: 'Approve', value: 'YES' }, { label: 'Hold', value: 'NO' }],
    }],
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/questions/gate-approve/answer`, async (route) => {
    answers.push(route.request().postDataJSON())
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ status: 'accepted', pipeline_id: run.run_id, question_id: 'gate-approve' }),
    })
  })
  await openRunsForSmokeTest(page, projectPath)

  // The waiting run opens on its status item, which holds the question.
  const pendingQuestionsPanel = page.getByTestId('run-status-item').getByTestId('run-pending-human-gates-panel')
  const journalPanel = page.getByTestId('run-journal-panel')

  await expect(pendingQuestionsPanel).toBeVisible()
  await expect(pendingQuestionsPanel).toContainText('Approve production deploy?')
  await page.getByTestId('run-journal-toggle').click()
  await expect(journalPanel).toBeVisible()
  await expect(journalPanel).toContainText('Deploy package uploaded to staging.')

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        return (globalThis as typeof globalThis & {
          __runEventSourceController?: {
            latestUrl(pattern: string): string | null
          }
        }).__runEventSourceController?.latestUrl('/workspace/api/live/events') ?? ''
      })
    })
    .toContain(`run_id=${run.run_id}`)

  // Older pages load on their own: visits need the whole journal.
  await expect(journalPanel).toContainText('Stage build completed')
  await expect(journalPanel).toContainText('Stage build started')
  expect(journalRequestUrls.some((url) => url.includes('before_sequence=4'))).toBe(true)

  await page.evaluate(({ runId }) => {
    ;(globalThis as typeof globalThis & {
      __runEventSourceController?: {
        emitLatest(pattern: string, payload: unknown): void
      }
    }).__runEventSourceController?.emitLatest('/workspace/api/live/events', {
      type: 'run.journal_entry',
      resource: { kind: 'run', id: runId },
      payload: {
        type: 'StageCompleted',
        sequence: 6,
        emitted_at: '2026-03-03T12:00:06Z',
        node_id: 'deploy',
        index: 4,
        outcome: 'success',
      },
    })
  }, { runId: run.run_id })

  await expect(journalPanel).toContainText('Stage deploy completed (success)')
  await page.screenshot({ path: screenshotPath('08c-runs-panel-journal-live-tail.png'), fullPage: true })

  // The question is answered in place on the status item.
  await page.getByTestId('run-visit-item-status').click()
  await pendingQuestionsPanel.getByTestId('run-pending-human-gate-answer-YES').click()
  await expect.poll(() => answers).toEqual([expect.objectContaining({ question_id: 'gate-approve', selected_value: 'YES' })])
})

test('run checkpoint refreshes on revisit for item 9.2-01', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-checkpoint-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-checkpoint-${Date.now()}`,
    flow_name: 'CheckpointFlow',
    current_node: 'implement',
  })
  let checkpointFetchCount = 0

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}/checkpoint`, async (route) => {
    checkpointFetchCount += 1
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        checkpoint: {
          current_node: 'implement',
          completed_nodes: ['start', 'plan'],
          retry_counts: { implement: 1 },
          timestamp: '2026-03-03T12:01:30Z',
        },
      }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)

  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(1)

  await page.getByRole('button', { name: 'All projects', exact: true }).click()
  await page.getByRole('button', { name: 'Active project', exact: true }).click()
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(2)
  await page.screenshot({ path: screenshotPath('08d-runs-panel-checkpoint-viewer.png'), fullPage: true })
})

test('run context viewer supports search, copy, and export actions for items 9.3-01 and 9.3-03', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-context-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-context-${Date.now()}`,
    flow_name: 'ContextFlow',
  })

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        context: {
          'graph.goal': 'Ship copy export',
          'context.owner': 'reviewer',
          'context.retries': 1,
        },
      }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)
  await page.getByTestId('run-visit-item-context').click()
  await page.evaluate(() => {
    Object.defineProperty(window.navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          ;(globalThis as typeof globalThis & { __copied_context_payload__?: string }).__copied_context_payload__ = value
        },
      },
    })
  })

  await expect(page.getByTestId('run-context-panel')).toBeVisible()
  await expect(page.getByTestId('run-context-row')).toHaveCount(2)
  // Keys outside context. are runtime bookkeeping, collapsed behind a count.
  await expect(page.getByTestId('run-context-runtime-group')).toContainText('1 system key')
  await page.getByTestId('run-context-search-input').fill('owner')
  await expect(page.getByTestId('run-context-row')).toHaveCount(1)
  await expect(page.getByTestId('run-context-row-value')).toContainText('reviewer')

  await page.getByTestId('run-context-copy-button').click()
  await expect(page.getByTestId('run-context-copy-status')).toContainText('Filtered context copied.')
  await expect
    .poll(() => page.evaluate(() => (globalThis as typeof globalThis & { __copied_context_payload__?: string }).__copied_context_payload__ || ''))
    .toContain(`"pipeline_id": "${run.run_id}"`)
  await expect(page.getByTestId('run-context-export-button')).toHaveAttribute('href', /data:application\/json/)
  await page.screenshot({ path: screenshotPath('08f-runs-panel-context-viewer.png'), fullPage: true })
})

test('run graph panel renders /pipelines/{id}/graph-preview output for item 9.5-02', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-graph-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-graph-${Date.now()}`,
    flow_name: 'GraphFlow',
  })

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}/graph-preview`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        status: 'ok',
        graph: {
          nodes: [
            { id: 'start', label: 'Start', shape: 'Mdiamond' },
            { id: 'review', label: 'Review', shape: 'box' },
            { id: 'done', label: 'Done', shape: 'Msquare' },
          ],
          edges: [
            { from: 'start', to: 'review', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            { from: 'review', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
          ],
        },
        diagnostics: [],
        errors: [],
      }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)

  const graphPanel = page.getByTestId('run-graph-panel')
  await expect(graphPanel).toHaveCount(0)
  await page.getByTestId('run-graph-toggle').click()
  await expect(graphPanel).toBeVisible()
  await expect(page.getByTestId('run-graph-canvas')).toBeVisible()
  await expect(page.locator('[data-testid="run-graph-canvas"] .react-flow__node')).toHaveCount(3)
  await graphPanel.scrollIntoViewIfNeeded()
  await graphPanel.screenshot({ path: screenshotPath('08n-runs-panel-run-graph.png') })
})

test('a failed run shows its failure, the visit it stopped in, and that visit\'s files', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-failed-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-failed-${Date.now()}`,
    flow_name: 'FailedFlow',
    status: 'failed',
    outcome: 'failure',
    last_error: 'codergen backend failed: codex app-server exited unexpectedly',
  })

  await stubRunSummary(page, run)
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/journal**`, {
    pipeline_id: run.run_id,
    entries: stageJournal([['start', 0, 'success'], ['implement', 1, 'running']]),
    oldest_sequence: 1,
    newest_sequence: 3,
    has_older: false,
  })
  await stubJson(page, `**/attractor/pipelines/${run.run_id}/artifacts`, {
    pipeline_id: run.run_id,
    artifacts: [
      { path: 'logs/implement/executions/1-0/prompt.md', size_bytes: 80, media_type: 'text/markdown', viewable: true },
    ],
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/artifacts/**`, async (route) => {
    await route.fulfill({
      status: 404,
      contentType: 'application/json',
      body: JSON.stringify({ detail: 'Artifact not found' }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)
  await expect(page.getByTestId('run-summary-retry-button')).toBeVisible()
  await expect(page.getByTestId('run-summary-cancel-button')).toHaveCount(0)

  await expect(page.getByTestId('run-status-failure')).toHaveText('codergen backend failed: codex app-server exited unexpectedly')
  await expect(page.getByTestId('run-status-failure-kind')).toHaveAttribute('data-kind', 'infrastructure')
  await expect(page.getByTestId('run-status-no-outputs')).toContainText('None: the run ended before it produced a result.')
  const stoppedIn = page.getByTestId('run-status-stopped-in').getByTestId('run-visit-link')
  await expect(stoppedIn).toHaveText('Implement')
  await stoppedIn.click()
  await expect(page.getByTestId('run-visit-view-title')).toHaveText('Implement')

  // The visit's files open in the artifact viewer, which reports a missing file.
  await page.getByTestId('run-visit-file').click()
  await expect(page.getByTestId('run-artifact-viewer-error')).toContainText(
    'Artifact preview unavailable because the file was not found for this run.',
  )
  await page.getByTestId('run-artifact-viewer').screenshot({ path: screenshotPath('08o-runs-panel-artifact-missing-partial.png') })
})

test('the run selector lists runs by title, folds child runs, and filters on search', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-selector-${Date.now()}`
  const parent = buildSmokeRun(projectPath, { run_id: `run-parent-${Date.now()}`, flow_name: 'implement-change.yaml', title: 'Fix the flaky webhook test' })
  const child = buildSmokeRun(projectPath, {
    run_id: `run-child-${Date.now()}`,
    flow_name: 'Implement Task',
    parent_run_id: parent.run_id,
    root_run_id: parent.run_id,
  })
  const other = buildSmokeRun(projectPath, { run_id: `run-other-${Date.now()}`, flow_name: 'merge-change.yaml', status: 'failed', title: 'Merge the Runs redesign' })

  await stubRunSummary(page, parent, [parent, child, other])
  await gotoWithRegisteredProject(page, projectPath)
  await page.getByTestId('nav-mode-runs').click()

  const titles = page.getByTestId('run-history-row-title')
  await expect(titles).toHaveText(['Fix the flaky webhook test', 'Merge the Runs redesign'])
  await expect(page.getByTestId('run-history-row-meta').nth(1)).toContainText('Merge Change')
  await expect(page.getByTestId('run-history-row-status')).toHaveText([' · Failed'])
  await page.getByTestId('run-history-children-toggle').click()
  await expect(titles).toHaveText(['Fix the flaky webhook test', 'Implement Task', 'Merge the Runs redesign'])
  await page.getByTestId('run-list-search-input').fill('merge')
  await expect(titles).toHaveText(['Merge the Runs redesign'])
})

test('run visits list a review loop and show one visit at a time', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-visits-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-visits-${Date.now()}`,
    flow_name: 'LoopFlow',
  })
  const stages: Array<[string, number, 'success' | 'fail']> = [
    ['start', 0, 'success'],
    ['implement', 1, 'success'],
    ['evaluate', 2, 'fail'],
    ['implement', 3, 'success'],
    ['evaluate', 4, 'success'],
    ['done', 5, 'success'],
  ]
  const journal = stages.flatMap(([node, index, outcome]) => [
    buildSmokeJournalEntry(index * 2 + 1, {
      kind: 'stage', raw_type: 'StageStarted', summary: `Stage ${node} started`, node_id: node, stage_index: index,
      emitted_at: `2026-03-03T12:0${index}:00Z`, payload: { node_id: node, index },
    }),
    buildSmokeJournalEntry(index * 2 + 2, {
      kind: 'stage',
      raw_type: outcome === 'fail' ? 'StageFailed' : 'StageCompleted',
      summary: `Stage ${node} ${outcome === 'fail' ? 'failed' : 'completed'}`,
      node_id: node,
      stage_index: index,
      emitted_at: `2026-03-03T12:0${index}:42Z`,
      payload: outcome === 'fail' ? { node_id: node, index, error: 'stage_failed' } : { node_id: node, index, outcome },
    }),
  ]).reverse()
  const rejection = {
    outcome: 'fail',
    failure_reason: 'Missing regression tests.',
    notes: '',
    context_updates: {
      'context.review.required_changes': 'Add a regression test for the loop.',
      'context.review.approved': null,
      last_stage: 'evaluate',
    },
  }
  const draft = { outcome: 'success', notes: '', context_updates: { 'context.review.approved': 'draft looks fine' } }
  const envelope = JSON.stringify({ outcome: 'fail', context_updates: rejection.context_updates })
  const segment = (id: string, order: number, content: string) => ({
    id, turn_id: 'response', order, kind: 'assistant_message', role: 'assistant', status: 'complete',
    timestamp: '2026-03-03T12:02:30Z', updated_at: '2026-03-03T12:02:30Z', content,
  })

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        ...run,
        completed_nodes: ['done'],
        progress: { current_node: 'done', completed_count: 6 },
        executions: stages.map(([node, index, outcome]) => ({
          run_id: run.run_id,
          node_id: node,
          stage_index: index,
          attempt: 0,
          status: index === 2 ? rejection : index === 1 ? draft : { outcome, notes: '', context_updates: {} },
        })),
        child_runs: [],
      }),
    })
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/executions/*/*/transcript`, async (route) => {
    const isRejection = new URL(route.request().url()).pathname.endsWith('/evaluate/2-0/transcript')
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        records: isRejection
          ? [
            { type: 'turn_upsert', turn: { id: 'prompt', role: 'user', kind: 'message', content: 'Judge the implementation against the contract.' } },
            { type: 'segment_upsert', source_event_sequence: 1, segment: segment('reply', 1, 'Checked the diff; the loop has no regression test.') },
            { type: 'segment_upsert', source_event_sequence: 2, segment: segment('final', 2, envelope) },
          ]
          : [],
      }),
    })
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/journal**`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ pipeline_id: run.run_id, entries: journal, oldest_sequence: 1, newest_sequence: 12, has_older: false }),
    })
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/graph-preview**`, async (route) => {
    const nodes = [
      { id: 'start', label: 'Start', kind: 'start' },
      { id: 'implement', label: 'Implement', kind: 'agent_task' },
      { id: 'evaluate', label: 'Evaluate', kind: 'agent_task' },
      { id: 'done', label: 'Done', kind: 'exit' },
    ]
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        status: 'ok',
        flow: {
          nodes: Object.fromEntries(nodes.map((node) => [node.id, {
            ...node,
            ...(node.id === 'evaluate' ? { contracts: { reads_context: ['context.task.objective'] } } : {}),
          }])),
        },
        graph: {
          nodes: nodes.map((node) => ({ id: node.id, label: node.label, shape: 'box' })),
          edges: [
            { from: 'start', to: 'implement' },
            { from: 'implement', to: 'evaluate' },
            { from: 'evaluate', to: 'implement' },
            { from: 'evaluate', to: 'done' },
          ].map((edge) => ({ ...edge, label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false })),
        },
        diagnostics: [],
        errors: [],
      }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)
  await expect(page.getByTestId('run-graph-panel')).toHaveCount(0)

  // One row per visit, n/x for repeated nodes, marks only where notable.
  const rows = page.getByTestId('run-visit-row')
  await expect(rows).toHaveCount(6)
  await expect(rows.getByTestId('run-visit-row-count')).toHaveText(['', '1/2', '1/2', '2/2', '2/2', ''])
  await expect(rows.nth(2).locator('[data-mark]')).toHaveCount(2)
  await expect(rows.nth(2).locator('[data-mark="did_not_pass"]')).toBeVisible()
  await expect(rows.nth(2).locator('[data-mark="loop_back"]')).toBeVisible()
  await expect(rows.nth(4).locator('[data-mark]')).toHaveCount(0)

  // The Evaluate visit that didn't pass: instructions, transcript, reason, writes.
  await rows.nth(2).click()
  const view = page.getByTestId('run-visit-view')
  await expect(view).toContainText('visit 1 of 2')
  await expect(page.getByTestId('run-visit-view-outcome')).toHaveText("Didn't pass")
  const instructions = page.getByTestId('run-visit-instructions')
  await expect(instructions).not.toHaveAttribute('open', '')
  await instructions.locator('summary').click()
  await expect(instructions).toContainText('Judge the implementation against the contract.')
  await expect(page.getByTestId('run-visit-reads')).toContainText('context.task.objective')
  await expect(page.getByTestId('run-visit-work')).toContainText('the loop has no regression test')
  await expect(page.getByTestId('run-visit-work')).not.toContainText('"outcome"')
  await expect(page.getByTestId('run-visit-reason')).toHaveText('Missing regression tests.')
  await expect(page.getByTestId('run-visit-next')).toHaveText('Sent back to Implement 2/2')
  await expect(page.locator('[data-testid="run-visit-write"][data-key="context.review.required_changes"]')).toContainText('Add a regression test for the loop.')
  await expect(page.locator('[data-testid="run-visit-write"][data-key="context.review.approved"]')).toContainText('cleared')
  await expect(page.getByTestId('run-visit-system-writes')).toHaveText('1 system key written')
  await page.screenshot({ path: screenshotPath('08d-runs-panel-visit-view.png'), fullPage: true })

  // The picker jumps between this node's visits.
  await page.getByTestId('run-visit-picker').getByRole('button', { name: 'Visit 2 of 2' }).click()
  await expect(view).toContainText('visit 2 of 2')
  await expect(page.getByTestId('run-visit-view-outcome')).toHaveText('Succeeded')

  // Keyboard: left/right step within the node, up/down and j/k move through the list.
  await rows.nth(4).focus()
  await page.keyboard.press('ArrowLeft')
  await expect(rows.nth(2)).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('ArrowUp')
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByTestId('run-visit-view-title')).toHaveText('Implement')
  await page.keyboard.press('j')
  await expect(rows.nth(2)).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('ArrowRight')
  await expect(rows.nth(4)).toHaveAttribute('aria-selected', 'true')

  // The graph stays hidden until toggled, by button or by g.
  await expect(page.getByTestId('run-graph-panel')).toHaveCount(0)
  await page.keyboard.press('g')
  await expect(page.getByTestId('run-graph-panel')).toBeVisible()
  await page.getByTestId('run-graph-toggle').click()
  await expect(page.getByTestId('run-graph-panel')).toHaveCount(0)

  // A written key opens the Context item at that key, with its history across visits.
  await rows.nth(2).click()
  await page.locator('[data-testid="run-visit-write"][data-key="context.review.approved"]').getByTestId('run-visit-context-key').click()
  await expect(page.getByTestId('run-visit-item-context')).toHaveAttribute('aria-selected', 'true')
  const approved = page.locator('[data-testid="run-context-row"][data-context-key="context.review.approved"]')
  await expect(approved).toContainText('cleared by Evaluate 1/2')
  // Opened from a visit, the key shows its history straight away.
  await expect(approved.getByTestId('run-context-history-toggle')).toHaveAttribute('aria-expanded', 'true')
  await expect(approved.getByTestId('run-context-history-entry')).toHaveText(['Implement 1/2:draft looks fine', 'Evaluate 1/2:cleared'])
})
