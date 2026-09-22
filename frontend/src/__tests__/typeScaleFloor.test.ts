import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { expect, it } from 'vitest'

// Type scale floor is 12px (text-xs). Arbitrary text-[Npx]/text-[Nrem] sizes and CSS font-size below 12px are banned.
it('keeps every font size at or above the 12px floor', () => {
    const srcRoot = resolve(process.cwd(), 'src')
    const violations: string[] = []
    for (const file of readdirSync(srcRoot, { recursive: true }) as string[]) {
        if (!/\.(tsx?|css)$/.test(file)) continue
        readFileSync(join(srcRoot, file), 'utf8').split('\n').forEach((line, index) => {
            const arbitrary = line.match(/text-\[[\d.]+(px|r?em)\]/)
            if (arbitrary) violations.push(`${file}:${index + 1} ${arbitrary[0]}`)
            for (const [match, value, unit] of line.matchAll(/font-size:\s*([\d.]+)(px|rem)/g)) {
                if (Number(value) * (unit === 'rem' ? 16 : 1) < 12) violations.push(`${file}:${index + 1} ${match}`)
            }
        })
    }
    expect(violations).toEqual([])
})
