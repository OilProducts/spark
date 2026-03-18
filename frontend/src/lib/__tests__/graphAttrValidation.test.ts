import {
  getToolHookCommandWarning,
  normalizeGraphAttrValue,
  validateGraphAttrValue,
} from '@/lib/graphAttrValidation'
import { describe, expect, it } from 'vitest'

describe('graphAttrValidation', () => {
  it('normalizes graph attrs using key-specific rules', () => {
    expect(normalizeGraphAttrValue('sparkspawn.title', '  Execution Planning  ')).toBe('Execution Planning')
    expect(normalizeGraphAttrValue('sparkspawn.description', '  Turn spec edits into plans.  ')).toBe(
      'Turn spec edits into plans.',
    )
    expect(normalizeGraphAttrValue('goal', '  Ship release  ')).toBe('Ship release')
    expect(normalizeGraphAttrValue('default_max_retry', ' 003 ')).toBe('3')
    expect(normalizeGraphAttrValue('default_max_retry', 'abc')).toBe('abc')
    expect(normalizeGraphAttrValue('default_fidelity', ' Summary:High ')).toBe('summary:high')
    expect(normalizeGraphAttrValue('model_stylesheet', '  .fast { llm_model: x; }  ')).toBe(
      '  .fast { llm_model: x; }  ',
    )
  })

  it('validates fidelity and retry constraints', () => {
    expect(validateGraphAttrValue('default_max_retry', '')).toBeNull()
    expect(validateGraphAttrValue('default_max_retry', '2')).toBeNull()
    expect(validateGraphAttrValue('default_max_retry', '-1')).toBe(
      'Default max retry must be a non-negative integer.',
    )

    expect(validateGraphAttrValue('default_fidelity', '')).toBeNull()
    expect(validateGraphAttrValue('default_fidelity', 'summary:medium')).toBeNull()
    expect(validateGraphAttrValue('default_fidelity', 'ultra')).toContain('Default fidelity must be one of')
  })

  it('warns for malformed tool hook commands', () => {
    expect(getToolHookCommandWarning('')).toBeNull()
    expect(getToolHookCommandWarning('echo pre')).toBeNull()
    expect(getToolHookCommandWarning('echo first\necho second')).toBe(
      'Tool hook command should be a single line shell command.',
    )
    expect(getToolHookCommandWarning("echo 'broken")).toBe(
      'Tool hook command appears malformed: unmatched single quote.',
    )
    expect(getToolHookCommandWarning('echo "broken')).toBe(
      'Tool hook command appears malformed: unmatched double quote.',
    )
  })
})
