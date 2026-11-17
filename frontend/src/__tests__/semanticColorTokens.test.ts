import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { render } from '@testing-library/react'
import { createElement } from 'react'
import { expect, it } from 'vitest'
import { StatusPill } from '@/components/app/status'

// Text tokens must meet WCAG AA (4.5:1) against the surfaces they sit on, in light and dark.
function tokens(css: string, selector: string) {
    const block = css.match(new RegExp(`${selector.replace('.', '\\.')} \\{([^}]*)\\}`))![1]
    return Object.fromEntries([...block.matchAll(/--([\w-]+):\s*([\d.]+) ([\d.]+)% ([\d.]+)%;/g)]
        .map(([, name, h, s, l]) => [name, rgb([Number(h), Number(s) / 100, Number(l) / 100])]))
}

function rgb([h, s, l]: number[]) {
    return [0, 8, 4].map((n) => {
        const k = (n + h / 30) % 12
        return l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1))
    })
}

function luminance([r, g, b]: number[]) {
    const linear = (c: number) => c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

function contrast(a: number[], b: number[]) {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
    return (hi + 0.05) / (lo + 0.05)
}

it.each([':root', '.dark'])('keeps text tokens at WCAG AA contrast in %s', (selector) => {
    const theme = tokens(readFileSync(resolve(process.cwd(), 'src/index.css'), 'utf8'), selector)
    const pairs = [
        ['foreground', 'background'], ['muted-foreground', 'background'], ['muted-foreground', 'muted'],
        ['foreground', 'ground'], ['muted-foreground', 'ground'],
        // SegmentedControl: inactive labels sit on its border-coloured track.
        ['muted-foreground', 'border'],
        ['primary', 'background'], ['accent-foreground', 'accent'], ['destructive', 'background'],
        ['success', 'background'], ['warning', 'background'], ['popover-foreground', 'popover'],
        ['success', 'ground'], ['warning', 'ground'], ['destructive', 'ground'], ['primary', 'ground'],
    ]
    const failures = pairs
        .map(([text, surface]) => [text, surface, contrast(theme[text], theme[surface])] as const)
        .filter(([, , ratio]) => !(ratio >= 4.5))
    expect(failures).toEqual([])

    // Resolve the rendered colors. An Interrupted warning pill can also sit inside a muted well.
    const pillFailures = (['success', 'warning', 'destructive', 'info'] as const).flatMap((tone) => {
        const { getByText } = render(createElement(StatusPill, { tone }, tone))
        const classes = [...getByText(tone).classList]
        const textClass = classes.find((name) => name.startsWith('text-') && Object.hasOwn(theme, name.slice(5)))
        const backgroundClass = classes.find((name) => name.startsWith('bg-'))
        if (!textClass || !backgroundClass) return [`${tone} pill is missing color tokens`]

        const [backgroundToken, opacity = '100'] = backgroundClass.slice(3).split('/')
        const alpha = Number(opacity) / 100
        const surfaces = tone === 'warning' ? ['background', 'muted'] : ['background']
        return surfaces.flatMap((surface) => {
            const background = theme[backgroundToken].map((c, i) => c * alpha + theme[surface][i] * (1 - alpha))
            const ratio = contrast(theme[textClass.slice(5)], background)
            return ratio >= 4.5 ? [] : [`${tone} pill on ${surface} ${ratio.toFixed(2)}`]
        })
    })
    expect(pillFailures).toEqual([])
})
