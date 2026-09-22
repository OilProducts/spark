import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { expect, it } from 'vitest'

// Status color comes from theme tokens (success/warning/info/destructive/muted in index.css), never raw Tailwind palette classes.
// StylesheetEditor is exempt: its dark code surface uses syntax-highlight colors.
const RAW_PALETTE = /\b(bg|text|border|ring|fill|stroke|from|to|caret)-(red|amber|yellow|emerald|green|blue|sky|indigo|violet|purple|orange|rose|slate|zinc|gray|neutral|stone|cyan|teal|lime|fuchsia|pink)-[0-9]+/g
const ALLOWLIST = ['features/editor/components/StylesheetEditor.tsx']

it('uses semantic color tokens instead of raw palette classes', () => {
    const srcRoot = resolve(process.cwd(), 'src')
    const violations: string[] = []
    for (const file of readdirSync(srcRoot, { recursive: true }) as string[]) {
        if (!/\.tsx?$/.test(file) || ALLOWLIST.includes(file.replaceAll('\\', '/'))) continue
        readFileSync(join(srcRoot, file), 'utf8').split('\n').forEach((line, index) => {
            for (const [match] of line.matchAll(RAW_PALETTE)) violations.push(`${file}:${index + 1} ${match}`)
        })
    }
    expect(violations).toEqual([])
})
