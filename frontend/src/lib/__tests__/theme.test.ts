import { afterEach, expect, it, vi } from 'vitest'
import { applyTheme } from '../theme'

afterEach(() => {
    applyTheme('light')
    document.documentElement.style.removeProperty('color-scheme')
    localStorage.removeItem('spark.appearance')
    vi.unstubAllGlobals()
})

it('follows the OS color scheme in System mode and honors explicit appearance choices', () => {
    const media = Object.assign(new EventTarget(), { matches: true })
    vi.stubGlobal('matchMedia', (query: string) => query === '(prefers-color-scheme: dark)' ? media : undefined)

    applyTheme('system')
    expect(document.documentElement).toHaveClass('dark')
    expect(document.documentElement.style.colorScheme).toBe('dark')

    media.matches = false
    media.dispatchEvent(new Event('change'))
    expect(document.documentElement).not.toHaveClass('dark')
    expect(document.documentElement.style.colorScheme).toBe('light')

    applyTheme('dark')
    expect(document.documentElement).toHaveClass('dark')
    expect(document.documentElement.style.colorScheme).toBe('dark')

    media.matches = true
    applyTheme('light')
    expect(document.documentElement).not.toHaveClass('dark')
    expect(document.documentElement.style.colorScheme).toBe('light')

    media.matches = false
    media.dispatchEvent(new Event('change'))
    media.matches = true
    media.dispatchEvent(new Event('change'))
    expect(document.documentElement).not.toHaveClass('dark')
    expect(document.documentElement.style.colorScheme).toBe('light')
})
