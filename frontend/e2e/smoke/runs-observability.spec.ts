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

async function stubRunSummary(page: Page, run: SmokeRunRecord) {
  await page.route('**/attractor/runs**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        runs: [run],
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
          completed_nodes: run.current_node ? [run.current_node] : [],
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

async function openRunsForSmokeTest(page: Page, projectPath: string) {
  await gotoWithRegisteredProject(page, projectPath)
  await page.getByTestId('nav-mode-runs').click()
  await expect(page.getByTestId('run-history-row').first()).toBeVisible()
  await page.getByTestId('run-history-row').first().click()
  await expect(page.getByTestId('run-summary-panel')).toBeVisible()
}

async function openRunsAdvancedEvidence(page: Page) {
  await expect(page.getByTestId('run-advanced-panel')).toBeVisible()
  await page.getByTestId('run-advanced-toggle-button').click()
}

test('run summary panel renders populated metadata for items 9.1-01 and 9.6-02', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-summary-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-summary-${Date.now()}`,
    flow_name: 'SmokeFlow',
    git_branch: 'feature/traceability',
    git_commit: 'fedcba9876543210',
  })

  await stubRunSummary(page, run)
  await openRunsForSmokeTest(page, projectPath)

  await expect(page.getByTestId('run-summary-panel')).toContainText(run.run_id)
  await expect(page.getByTestId('run-summary-status')).toContainText('Completed')
  await expect(page.getByTestId('run-summary-outcome')).toContainText('Success')
  await expect(page.getByTestId('run-summary-flow-name')).toContainText('SmokeFlow')
  await expect(page.getByTestId('run-summary-model')).toContainText('gpt-5')
  await expect(page.getByTestId('run-summary-working-directory')).toContainText(`${projectPath}/workspace`)
  await expect(page.getByTestId('run-summary-project-path')).toContainText(projectPath)
  await expect(page.getByTestId('run-summary-git-branch')).toContainText('feature/traceability')
  await expect(page.getByTestId('run-summary-git-commit')).toContainText('fedcba9876543210')
  await expect(page.getByTestId('run-summary-estimated-model-cost')).toContainText('$0.000257')
  await expect(page.getByTestId('run-summary-token-usage')).toContainText('42')
  await expect(page.getByTestId('run-summary-model-breakdown')).toContainText('gpt-5.4')
  await page.screenshot({ path: screenshotPath('08b-runs-panel-populated-summary.png'), fullPage: true })
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

  await openRunsForSmokeTest(page, projectPath)

  const pendingQuestionsPanel = page.getByTestId('run-pending-human-gates-panel')
  const journalPanel = page.getByTestId('run-event-timeline-panel')

  await expect(pendingQuestionsPanel).toBeVisible()
  await expect(pendingQuestionsPanel).toContainText('Approve production deploy?')
  await expect(page.getByTestId('run-summary-now-pending-questions')).toContainText('1')
  await expect(page.getByTestId('run-summary-now-latest-journal')).toContainText('Human gate pending: Approve production deploy?')
  await expect(journalPanel).toBeVisible()
  await expect(journalPanel).toContainText('Deploy package uploaded to staging.')

  const [pendingQuestionsBox, journalBox] = await Promise.all([
    pendingQuestionsPanel.boundingBox(),
    journalPanel.boundingBox(),
  ])
  expect(pendingQuestionsBox?.y).not.toBeNull()
  expect(journalBox?.y).not.toBeNull()
  expect((pendingQuestionsBox?.y ?? Number.POSITIVE_INFINITY)).toBeLessThan(journalBox?.y ?? Number.NEGATIVE_INFINITY)

  await expect
    .poll(async () => {
      return page.evaluate((runId) => {
        return (globalThis as typeof globalThis & {
          __runEventSourceController?: {
            latestUrl(pattern: string): string | null
          }
        }).__runEventSourceController?.latestUrl(`/attractor/pipelines/${runId}/events`) ?? ''
      }, run.run_id)
    })
    .toContain(`after_sequence=${latestEntries[0]!.sequence}`)

  await expect(page.getByTestId('run-journal-load-older')).toBeVisible()
  await page.getByTestId('run-journal-load-older').click()

  await expect(page.getByTestId('run-event-timeline-list')).toContainText('Stage build completed')
  await expect(page.getByTestId('run-event-timeline-list')).toContainText('Stage build started')
  await expect(page.getByTestId('run-journal-load-older')).toHaveCount(0)
  expect(journalRequestUrls.some((url) => url.includes('before_sequence=4'))).toBe(true)

  await page.evaluate(({ runId }) => {
    ;(globalThis as typeof globalThis & {
      __runEventSourceController?: {
        emitLatest(pattern: string, payload: unknown): void
      }
    }).__runEventSourceController?.emitLatest(`/attractor/pipelines/${runId}/events`, {
      type: 'StageCompleted',
      sequence: 6,
      emitted_at: '2026-03-03T12:00:06Z',
      node_id: 'deploy',
      index: 4,
      outcome: 'success',
    })
  }, { runId: run.run_id })

  await expect(page.getByTestId('run-summary-now-latest-journal')).toContainText('Stage deploy completed (success)')
  await expect(page.getByTestId('run-event-timeline-list')).toContainText('Stage deploy completed (success)')
  await page.screenshot({ path: screenshotPath('08c-runs-panel-journal-live-tail.png'), fullPage: true })
})

test('run checkpoint viewer fetches checkpoint payload for item 9.2-01', async ({ page }) => {
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
  await openRunsAdvancedEvidence(page)

  await expect(page.getByTestId('run-checkpoint-panel')).toBeVisible()
  await expect(page.getByTestId('run-checkpoint-payload')).toContainText('"current_node": "implement"')
  await expect(page.getByTestId('run-checkpoint-payload')).toContainText('"retry_counts":')
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(1)

  await page.getByTestId('run-checkpoint-refresh-button').click()
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
          owner: 'reviewer',
          retries: 1,
        },
      }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)
  await openRunsAdvancedEvidence(page)
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
  await expect(page.getByTestId('run-context-table')).toBeVisible()
  await page.getByTestId('run-context-search-input').fill('owner')
  await expect(page.getByTestId('run-context-row')).toHaveCount(1)
  await expect(page.getByTestId('run-context-row-value-scalar')).toContainText('reviewer')

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
          graph_attrs: {
            label: 'Run graph smoke',
          },
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
  await openRunsAdvancedEvidence(page)

  const graphPanel = page.getByTestId('run-graph-panel')
  await expect(graphPanel).toBeVisible()
  await page.getByTestId('run-graph-toggle-button').click()
  await expect(page.getByTestId('run-graph-canvas')).toBeVisible()
  await expect(page.locator('[data-testid="run-graph-canvas"] .react-flow__node')).toHaveCount(3)
  await graphPanel.scrollIntoViewIfNeeded()
  await graphPanel.screenshot({ path: screenshotPath('08n-runs-panel-run-graph.png') })
})

test('run artifact browser handles missing files and partial run states for item 9.5-03', async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-artifacts-missing-${Date.now()}`
  const run = buildSmokeRun(projectPath, {
    run_id: `run-artifacts-missing-${Date.now()}`,
    flow_name: 'ArtifactMissingFlow',
    status: 'failed',
    outcome: null,
    git_commit: 'art9503',
    last_error: 'stage artifact missing',
    token_usage: 9,
  })

  await stubRunSummary(page, run)
  await page.route(`**/attractor/pipelines/${run.run_id}/artifacts`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pipeline_id: run.run_id,
        artifacts: [
          {
            path: 'plan/prompt.md',
            size_bytes: 80,
            media_type: 'text/markdown',
            viewable: true,
          },
        ],
      }),
    })
  })
  await page.route(`**/attractor/pipelines/${run.run_id}/artifacts/**`, async (route) => {
    const url = new URL(route.request().url())
    if (url.pathname.endsWith(`/pipelines/${run.run_id}/artifacts`)) {
      await route.fallback()
      return
    }
    await route.fulfill({
      status: 404,
      contentType: 'application/json',
      body: JSON.stringify({ detail: 'Artifact not found' }),
    })
  })

  await openRunsForSmokeTest(page, projectPath)
  await openRunsAdvancedEvidence(page)

  const artifactPanel = page.getByTestId('run-artifact-panel')
  await expect(artifactPanel).toBeVisible()
  await expect(page.getByTestId('run-artifact-partial-run-note')).toContainText(
    'This run may be partial or artifacts may have been pruned.',
  )
  await expect(page.getByTestId('run-artifact-partial-run-note')).toContainText(
    'Missing expected files: manifest.json, checkpoint.json.',
  )

  const promptRow = page.getByTestId('run-artifact-row').filter({ hasText: 'plan/prompt.md' }).first()
  await promptRow.getByTestId('run-artifact-view-button').click()
  await expect(page.getByTestId('run-artifact-viewer-error')).toContainText(
    'Artifact preview unavailable because the file was not found for this run.',
  )

  await artifactPanel.scrollIntoViewIfNeeded()
  await artifactPanel.screenshot({ path: screenshotPath('08o-runs-panel-artifact-missing-partial.png') })
})
