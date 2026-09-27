import { resolveModelStylesheetPreview } from '@/lib/modelStylesheetPreview'
import { describe, expect, it } from 'vitest'

describe('modelStylesheetPreview', () => {
  it('accepts xhigh reasoning effort declarations', () => {
    const preview = resolveModelStylesheetPreview(
      '* { reasoning_effort: xhigh; }',
      [{ id: 'plan', shape: 'box' }],
      {},
    )

    expect(preview.nodePreview[0].effective.reasoning_effort).toEqual({
      value: 'xhigh',
      source: 'stylesheet',
    })
  })
})

it('resolves native controls and arbitrary effort through node, stylesheet, and graph defaults', () => {
  const result = resolveModelStylesheetPreview('* { reasoning_effort: future; reasoning_summary: detailed; thinking: budget; thinking_budget_tokens: 2048; }',
    [{ id: 'work', reasoning_summary: 'concise' }], { reasoning_mode: 'pro' }).nodePreview[0].effective
  expect(result.reasoning_effort).toEqual({ value: 'future', source: 'stylesheet' })
  expect(result.reasoning_summary).toEqual({ value: 'concise', source: 'node' })
  expect(result.thinking_budget_tokens).toEqual({ value: '2048', source: 'stylesheet' })
  expect(result.reasoning_mode).toEqual({ value: 'pro', source: 'graph_default' })
})
