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

/**
 * Unified-diff-style text with added and removed lines marked. It is a named tab stop
 * so the keyboard can scroll what it clips; WebKit does not focus scroll boxes itself.
 */
export function DiffBlock({ diff, label, className, testId }: { diff: string; label: string; className?: string; testId?: string }) {
    if (!diff.trim()) return <p className="text-xs text-muted-foreground">No differences.</p>
    const lines = diff.replace(/\n$/, '').split('\n')
    const tones = lineTones(lines)
    return <pre
        data-testid={testId}
        role="group"
        aria-label={label}
        tabIndex={0}
        className={cn('max-h-96 overflow-auto rounded-md border bg-muted/40 p-2 text-xs leading-5 outline-none focus-visible:ring-2 focus-visible:ring-ring', className)}
    >
        {lines.map((line, index) => <div key={index} className={tones[index]}>{line || ' '}</div>)}
    </pre>
}
