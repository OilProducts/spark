import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { expect, it } from 'vitest'

// Status color comes from theme tokens (success/warning/info/destructive/muted in index.css), never raw Tailwind palette classes.
const RAW_PALETTE = /\b(bg|text|border|ring|fill|stroke|from|to|caret)-(red|amber|yellow|emerald|green|blue|sky|indigo|violet|purple|orange|rose|slate|zinc|gray|neutral|stone|cyan|teal|lime|fuchsia|pink)-[0-9]+/g

it('uses semantic color tokens instead of raw palette classes', () => {
    const srcRoot = resolve(process.cwd(), 'src')
    const violations: string[] = []
    for (const file of readdirSync(srcRoot, { recursive: true }) as string[]) {
        if (!/\.tsx?$/.test(file)) continue
        readFileSync(join(srcRoot, file), 'utf8').split('\n').forEach((line, index) => {
            for (const [match] of line.matchAll(RAW_PALETTE)) violations.push(`${file}:${index + 1} ${match}`)
        })
    }
    expect(violations).toEqual([])
})

// Inline errors render through InlineError (Alert variant="destructive"); hand-rolled tinted destructive boxes are banned.
it('keeps ad hoc destructive error boxes out of feature code', () => {
    const srcRoot = resolve(process.cwd(), 'src')
    const violations: string[] = []
    for (const file of readdirSync(srcRoot, { recursive: true }) as string[]) {
        if (!file.endsWith('.tsx') || file.replaceAll('\\', '/').startsWith('components/ui/')) continue
        readFileSync(join(srcRoot, file), 'utf8').split('\n').forEach((line, index) => {
            const box = line.match(/border-destructive\/\d+ bg-destructive\/\d+/)
            if (box) violations.push(`${file}:${index + 1} ${box[0]}`)
        })
    }
    expect(violations).toEqual([])
})

// Text tokens must meet WCAG AA (4.5:1) against the surfaces they sit on, in light and dark.
function tokens(css: string, selector: string) {
    const block = css.match(new RegExp(`${selector.replace('.', '\\.')} \\{([^}]*)\\}`))![1]
    return Object.fromEntries([...block.matchAll(/--([\w-]+):\s*([\d.]+) ([\d.]+)% ([\d.]+)%;/g)]
        .map(([, name, h, s, l]) => [name, [Number(h), Number(s) / 100, Number(l) / 100]]))
}

function luminance([h, s, l]: number[]) {
    const f = (n: number) => {
        const k = (n + h / 30) % 12
        const c = l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1))
        return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
    }
    return 0.2126 * f(0) + 0.7152 * f(8) + 0.0722 * f(4)
}

function contrast(a: number[], b: number[]) {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
    return (hi + 0.05) / (lo + 0.05)
}

it.each([':root', '.dark'])('keeps text tokens at WCAG AA contrast in %s', (selector) => {
    const theme = tokens(readFileSync(resolve(process.cwd(), 'src/index.css'), 'utf8'), selector)
    const pairs = [
        ['foreground', 'background'], ['muted-foreground', 'background'], ['muted-foreground', 'muted'],
        ['primary', 'background'], ['accent-foreground', 'accent'], ['destructive', 'background'],
        ['success', 'background'], ['warning', 'background'], ['popover-foreground', 'popover'],
    ]
    const failures = pairs
        .map(([text, surface]) => [text, surface, contrast(theme[text], theme[surface]).toFixed(2)] as const)
        .filter(([, , ratio]) => Number(ratio) < 4.5)
    expect(failures).toEqual([])
})
