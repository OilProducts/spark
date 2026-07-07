import {
  getToolHookCommandWarning,
  normalizeGraphAttrValue,
  validateGraphAttrValue,
} from '@/lib/graphAttrValidation'
import { describe, expect, it } from 'vitest'

describe('graphAttrValidation', () => {
  it('normalizes graph attrs using key-specific rules', () => {
    expect(normalizeGraphAttrValue('title', '  Implement From Plan File  ')).toBe(
      'Implement From Plan File',
    )
    expect(
      normalizeGraphAttrValue(
        'description',
        '  Snapshot a plan file, implement it, and iterate until complete.  ',
      ),
    ).toBe(
      'Snapshot a plan file, implement it, and iterate until complete.',
    )
    expect(normalizeGraphAttrValue('goal', '  Ship release  ')).toBe('Ship release')
    expect(normalizeGraphAttrValue('max_retries', ' 003 ')).toBe('3')
    expect(normalizeGraphAttrValue('max_retries', 'abc')).toBe('abc')
    expect(normalizeGraphAttrValue('fidelity', ' Summary:High ')).toBe('summary:high')
  })

  it('validates fidelity and retry constraints', () => {
    expect(validateGraphAttrValue('max_retries', '')).toBeNull()
    expect(validateGraphAttrValue('max_retries', '2')).toBeNull()
    expect(validateGraphAttrValue('max_retries', '-1')).toBe(
      'Max retries default must be a non-negative integer.',
    )

    expect(validateGraphAttrValue('fidelity', '')).toBeNull()
    expect(validateGraphAttrValue('fidelity', 'summary:medium')).toBeNull()
    expect(validateGraphAttrValue('fidelity', 'ultra')).toContain('Fidelity default must be one of')
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
