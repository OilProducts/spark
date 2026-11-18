import { describe, expect, it } from 'vitest'

import { parseContextKeyDraft, parseContextKeyList, parseLaunchInputDefinitions } from '@/lib/flowContracts'

describe('context keys are literal', () => {
  it('reports a padded or malformed key instead of trimming it', () => {
    expect(parseContextKeyList('[" context.plan"]').error).toContain('" context.plan"')
    expect(parseContextKeyDraft('context.plan \ncontext.review').error).toContain('"context.plan "')
    expect(parseContextKeyDraft('context..plan').error).toContain('empty segments')
    expect(parseLaunchInputDefinitions([{ key: 'context.request ', label: 'Request', type: 'string' }]).error)
      .toContain('"context.request "')
  })

  it('accepts context.* and Spark namespaces, and rejects any other namespace', () => {
    expect(parseContextKeyList('["context.plan", "tool.output", "stack.child.status", "parallel.results"]')).toEqual({
      keys: ['context.plan', 'tool.output', 'stack.child.status', 'parallel.results'],
      error: null,
    })
    for (const key of ['work.plan', 'review.summary', 'outcome']) {
      expect(parseContextKeyDraft(key).error).toContain(`namespace or one Spark supplies`)
    }
  })
})
