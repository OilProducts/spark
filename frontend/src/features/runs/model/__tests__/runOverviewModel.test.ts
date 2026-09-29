import { describe, expect, it } from 'vitest'

import {
  buildRunContextOverview,
  classifyRunFailure,
  didNotPassVisits,
  recordedGitRef,
  runTitle,
} from '../runOverviewModel'
import type { RunVisit, VisitOutcome } from '../visitModel'

const visit = (nodeId: string, number: number, outcome: VisitOutcome, updates: Record<string, unknown> | null = null): RunVisit => ({
  key: `${nodeId}-${number}`,
  runId: 'run-1',
  nodeId,
  stageIndex: number,
  attempt: 0,
  label: nodeId,
  kind: 'agent_task',
  number,
  count: 1,
  startedAt: null,
  endedAt: null,
  outcome,
  reason: null,
  status: updates ? { outcome: 'success', context_updates: updates } : null,
  next: null,
  childRun: null,
  parentKey: null,
})

describe('buildRunContextOverview', () => {
  it('keeps each key history across visits, including a clear, grouped by namespace', () => {
    const review1 = visit('review', 1, 'did_not_pass', { 'context.review.findings': 'missing test' })
    const evaluate1 = visit('evaluate', 2, 'succeeded', { 'context.review.findings': null, 'context.review.passed': false })
    const review2 = visit('review', 3, 'succeeded', { 'context.review.findings': 'still missing test', '_attractor.runtime.x': 1 })
    const overview = buildRunContextOverview({
      visits: [review1, evaluate1, review2],
      launchContext: { 'context.task.artifact_path': 'changes/CR-1/request.md' },
      finalContext: {
        'context.review.findings': 'still missing test',
        'context.review.passed': false,
        'context.task.artifact_path': 'changes/CR-1/request.md',
        'context.stack.child.depth': 1,
        outcome: 'success',
      },
    })

    expect(overview.namespaces.map((namespace) => namespace.name)).toEqual(['review', 'task'])
    const findings = overview.namespaces[0].keys.find((key) => key.key === 'context.review.findings')!
    expect(findings.value).toBe('still missing test')
    expect(findings.history.map((entry) => [entry.visit?.key ?? 'launch', entry.value])).toEqual([
      ['review-1', 'missing test'],
      ['evaluate-2', null],
      ['review-3', 'still missing test'],
    ])

    const input = overview.namespaces[1].keys[0]
    expect(input).toMatchObject({ key: 'context.task.artifact_path', value: 'changes/CR-1/request.md' })
    expect(input.history).toEqual([{ value: 'changes/CR-1/request.md', visit: null }])

    // System keys are counted, not listed among the namespaces.
    expect(overview.systemKeys).toEqual(['_attractor.runtime.x', 'context.stack.child.depth', 'outcome'])
    expect(overview.namespaces.flatMap((namespace) => namespace.keys.map((key) => key.key))).not.toContain('outcome')
  })

  it('shows a key the final context no longer has as cleared', () => {
    const overview = buildRunContextOverview({
      visits: [visit('review', 1, 'succeeded', { 'context.review.findings': 'x' }), visit('evaluate', 2, 'succeeded', { 'context.review.findings': null })],
      launchContext: null,
      finalContext: {},
    })
    expect(overview.namespaces[0].keys[0]).toMatchObject({ key: 'context.review.findings', value: null })
  })

  it('follows the latest visit write over an older snapshot: set, clear, set', () => {
    const review1 = visit('review', 1, 'did_not_pass', { 'context.review.findings': 'missing test' })
    const evaluate = visit('evaluate', 2, 'succeeded', { 'context.review.findings': null })
    const review2 = visit('review', 3, 'did_not_pass', { 'context.review.findings': 'still missing' })
    // The snapshot was taken after the first review; later writes arrive live.
    const stale = { 'context.review.findings': 'missing test', 'context.workspace.commit': 'abc1234' }
    const value = (visits: RunVisit[]) => buildRunContextOverview({ visits, launchContext: null, finalContext: stale })
      .namespaces.find((namespace) => namespace.name === 'review')!.keys[0]
    expect(value([review1, evaluate])).toMatchObject({ value: null })
    expect(value([review1, evaluate]).history.at(-1)?.visit).toBe(evaluate)
    expect(value([review1, evaluate, review2]).value).toBe('still missing')
    // A key no visit wrote keeps its snapshot value; a parallel branch's write defers to it too.
    const branch = { ...visit('lint', 4, 'succeeded', { 'context.workspace.commit': 'branch1' }), parentKey: 'fanout' }
    const overview = buildRunContextOverview({ visits: [review1, branch], launchContext: null, finalContext: stale })
    expect(recordedGitRef(overview)).toEqual({ commit: 'abc1234', branch: null })
  })
})

describe('didNotPassVisits', () => {
  it('lists every visit that did not pass, in run order', () => {
    const visits = [
      visit('review', 1, 'did_not_pass'),
      visit('evaluate', 2, 'succeeded'),
      visit('review', 3, 'did_not_pass'),
      visit('commit', 4, 'failed'),
    ]
    expect(didNotPassVisits(visits).map((entry) => entry.key)).toEqual(['review-1', 'review-3'])
  })
})

describe('classifyRunFailure', () => {
  it.each([
    ['codergen backend failed: codex app-server error: codex app-server turn timed out waiting for activity', 'infrastructure'],
    ['Unable to start execution container from image spark-mathlab:latest: failed to connect to the docker API', 'infrastructure'],
    ['codergen backend failed: usage limit reached', 'infrastructure'],
    ['The binding task source changes/CR-2026-0001/request.md is missing from both the working directory and HEAD.', 'flow'],
    ['max retries exceeded', 'flow'],
    ['', 'flow'],
  ])('%s is a %s failure', (lastError, kind) => {
    expect(classifyRunFailure({ last_error: lastError })).toBe(kind)
  })
})

describe('runTitle', () => {
  it('uses the generated title, else the flow title and first launch input', () => {
    expect(runTitle({ title: 'Tighten the review loop', flow_name: 'implement-change.yaml' })).toBe('Tighten the review loop')
    expect(runTitle({
      title: null,
      flow_name: 'software-development/implement-change.yaml',
      launch_context: { 'context.request.artifact_path': 'changes/CR-1/request.md', 'context.request.validation_command': 'just test' },
    })).toBe('Implement Change · changes/CR-1/request.md')
    expect(runTitle({ title: '  ', flow_name: 'Implement Task', launch_context: null })).toBe('Implement Task')
    // The runs list carries only the first input.
    expect(runTitle({ flow_name: 'merge-change.yaml', first_launch_input: 'changes/CR-2/request.md' })).toBe('Merge Change · changes/CR-2/request.md')
  })
})
