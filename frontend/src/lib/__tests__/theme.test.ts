import { describe, expect, it } from 'vitest'
import { resolveTheme } from '../theme'

describe('resolveTheme', () => {
    it.each([
        ['system', true, 'dark'],
        ['system', false, 'light'],
        ['light', true, 'light'],
        ['light', false, 'light'],
        ['dark', true, 'dark'],
        ['dark', false, 'dark'],
    ] as const)('%s with system dark=%s resolves to %s', (preference, systemPrefersDark, expected) => {
        expect(resolveTheme(preference, systemPrefersDark)).toBe(expected)
    })
})
