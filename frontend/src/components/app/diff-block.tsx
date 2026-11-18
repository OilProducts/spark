import { cn } from '@/lib/utils'

/**
 * Each line's colour. Inside a hunk a line goes by its first character, so a removed
 * `-- x` is still a removal; a file's headers before its first `@@` are muted metadata.
 * Text without any `@@` is all hunk.
 */
function lineTones(lines: string[]) {
    let inHunk = !lines.some((line) => line.startsWith('@@'))
    return lines.map((line) => {
        // A hunk line starts with a space, `+`, `-` or `\`, so `diff ` opens the next file.
        if (line.startsWith('diff ')) inHunk = false
        if (line.startsWith('@@')) inHunk = true
        return !inHunk || line.startsWith('@@') ? 'text-muted-foreground'
            : line.startsWith('+') ? 'text-success'
                : line.startsWith('-') ? 'text-destructive'
                    : undefined
    })
}

/** Unified-diff-style text with added and removed lines marked. */
export function DiffBlock({ diff, className, testId }: { diff: string; className?: string; testId?: string }) {
    if (!diff.trim()) return <p className="text-xs text-muted-foreground">No differences.</p>
    const lines = diff.replace(/\n$/, '').split('\n')
    const tones = lineTones(lines)
    return <pre data-testid={testId} className={cn('max-h-96 overflow-auto rounded-md border bg-muted/40 p-2 text-xs leading-5', className)}>
        {lines.map((line, index) => <div key={index} className={tones[index]}>{line || ' '}</div>)}
    </pre>
}
