import { useMemo, useRef, type UIEvent } from 'react'

type TokenType = 'selector' | 'property' | 'value' | 'punctuation' | 'text'

interface TokenSegment {
    content: string
    type: TokenType
}

interface StylesheetEditorProps {
    value: string
    onChange: (value: string) => void
}

const TOKEN_STYLES: Record<TokenType, string> = {
    selector: 'text-sky-300',
    property: 'text-amber-300',
    value: 'text-emerald-300',
    punctuation: 'text-slate-400',
    text: 'text-slate-200',
}

const SELECTOR_PATTERN = /(#[A-Za-z_][\w-]*|\.[A-Za-z_][\w-]*|\*)/g
const PROPERTY_PATTERN = /^(\s*)([A-Za-z_][\w.]*)(\s*:\s*)([^;{}]*)(\s*;?\s*)$/

function highlightSelectors(source: string): TokenSegment[] {
    const tokens: TokenSegment[] = []
    let cursor = 0

    for (const match of source.matchAll(SELECTOR_PATTERN)) {
        const index = match.index ?? 0
        if (index > cursor) {
            tokens.push({ content: source.slice(cursor, index), type: 'text' })
        }
        tokens.push({ content: match[0], type: 'selector' })
        cursor = index + match[0].length
    }

    if (cursor < source.length) {
        tokens.push({ content: source.slice(cursor), type: 'text' })
    }

    if (tokens.length === 0) {
        tokens.push({ content: source, type: 'text' })
    }

    return tokens
}

function highlightLine(line: string): TokenSegment[] {
    if (line.length === 0) {
        return [{ content: '', type: 'text' }]
    }

    const propertyMatch = line.match(PROPERTY_PATTERN)
    if (propertyMatch) {
        const [, leading, property, separator, value, trailing] = propertyMatch
        return [
            { content: leading, type: 'text' },
            { content: property, type: 'property' },
            { content: separator, type: 'punctuation' },
            { content: value, type: 'value' },
            { content: trailing, type: 'punctuation' },
        ]
    }

    if (line.includes('{')) {
        const braceIndex = line.indexOf('{')
        const selectorPart = line.slice(0, braceIndex)
        return [
            ...highlightSelectors(selectorPart),
            { content: '{', type: 'punctuation' },
            { content: line.slice(braceIndex + 1), type: 'text' },
        ]
    }

    if (line.includes('}')) {
        const braceIndex = line.indexOf('}')
        return [
            { content: line.slice(0, braceIndex), type: 'text' },
            { content: '}', type: 'punctuation' },
            { content: line.slice(braceIndex + 1), type: 'text' },
        ]
    }

    return highlightSelectors(line)
}

export function StylesheetEditor({ value, onChange }: StylesheetEditorProps) {
    const highlightRef = useRef<HTMLPreElement | null>(null)
    const highlightedLines = useMemo(() => value.split('\n').map(highlightLine), [value])

    const handleScroll = (event: UIEvent<HTMLTextAreaElement>) => {
        if (!highlightRef.current) {
            return
        }

        highlightRef.current.scrollTop = event.currentTarget.scrollTop
        highlightRef.current.scrollLeft = event.currentTarget.scrollLeft
    }

    return (
        <div className="relative h-20" data-testid="model-stylesheet-editor">
            <pre
                ref={highlightRef}
                aria-hidden="true"
                data-testid="model-stylesheet-editor-highlight"
                className="pointer-events-none h-full w-full overflow-auto rounded-md border border-input bg-slate-950/95 px-2 py-1 text-xs leading-5 font-mono"
            >
                {highlightedLines.map((line, lineIndex) => (
                    <span key={`line-${lineIndex}`}>
                        {line.map((segment, segmentIndex) => (
                            <span
                                key={`line-${lineIndex}-segment-${segmentIndex}`}
                                className={TOKEN_STYLES[segment.type]}
                                data-token-type={segment.type}
                            >
                                {segment.content}
                            </span>
                        ))}
                        {lineIndex < highlightedLines.length - 1 ? '\n' : ''}
                    </span>
                ))}
            </pre>
            <textarea
                value={value}
                onChange={(event) => onChange(event.target.value)}
                onScroll={handleScroll}
                spellCheck={false}
                className="absolute inset-0 h-full w-full resize-none rounded-md border border-input bg-transparent px-2 py-1 text-xs leading-5 font-mono text-transparent caret-slate-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                style={{ WebkitTextFillColor: 'transparent' }}
            />
        </div>
    )
}
