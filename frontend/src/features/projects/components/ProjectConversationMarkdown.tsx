import { createContext, memo, useContext, useId, useMemo, useState, type ComponentProps } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import rehypeHighlight from 'rehype-highlight'
import type { Root, RootContent } from 'mdast'
import type { Root as HastRoot, Element } from 'hast'
import { isTauri } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { TranscriptCopyButton } from '@/components/app/transcript/TranscriptCopyButton'
import { MarkdownMermaid } from './MarkdownMermaid'
import 'katex/dist/katex.min.css'
import 'highlight.js/styles/github-dark.css'
import './conversation-markdown.css'

const MarkdownContext = createContext(false)

function footnoteLabels(prefix: string) {
    return (tree: HastRoot) => {
        const walk = (parent: HastRoot | Element) => {
            for (const node of parent.children) {
                if (node.type !== 'element') continue
                if (node.properties.id === 'footnote-label') node.properties.id = `${prefix}footnote-label`
                if (Array.isArray(node.properties.ariaDescribedBy)) {
                    node.properties.ariaDescribedBy = node.properties.ariaDescribedBy.map((id) => id === 'footnote-label' ? `${prefix}${id}` : id)
                }
                walk(node)
            }
        }
        walk(tree)
    }
}

function safeBreaksAndCode(this: { parse: (source: string) => unknown }) {
    const parse = this.parse.bind(this)
    return (tree: Root, file: { value: unknown }) => {
        const source = String(file.value)
        // Reparse the whole document so recovered references share its definitions.
        // Equal-length placeholders keep original offsets intact for code copying.
        let marker = '\uE000'
        while (source.includes(marker) || new RegExp(`&#(?:x0*${marker.charCodeAt(0).toString(16)}|0*${marker.charCodeAt(0)});?`, 'i').test(source)) {
            marker = String.fromCharCode(marker.charCodeAt(0) + 1)
        }
        const blockBreaks = new Set<number>()
        let recoveredSource = source
        const recoverBreaks = (node: Root | RootContent, parent?: Root | RootContent): boolean => {
            let changed = false
            if (node.type === 'html') {
                const tag = node.value.match(/^<br(?: ?\/)?>/i)?.[0]
                const offset = node.position?.start.offset
                if (tag && offset !== undefined) {
                    const standalone = parent && ['root', 'blockquote', 'listItem', 'footnoteDefinition'].includes(parent.type) && /^<br(?: ?\/)?>[ \t]*(?:\r?\n|$)/i.test(node.value)
                    const placeholder = standalone ? '***'.padEnd(tag.length) : marker + 'b'.repeat(tag.length - 2) + marker
                    if (standalone) blockBreaks.add(offset)
                    recoveredSource = recoveredSource.slice(0, offset) + placeholder + recoveredSource.slice(offset + tag.length)
                    changed = true
                }
            }
            if ('children' in node) {
                for (const child of node.children) changed = recoverBreaks(child, node) || changed
            }
            return changed
        }
        // ponytail: reparse per newly exposed HTML block; batch only if large messages need it.
        while (recoverBreaks(tree)) tree.children = (parse(recoveredSource) as Root).children
        const breakPattern = new RegExp(`(${marker}b{2,4}${marker})`, 'g')
        const definitions = new Map<string, string>()
        const collectDefinitions = (node: Root | RootContent) => {
            if (node.type === 'definition' && !definitions.has(node.identifier)) definitions.set(node.identifier, node.url)
            if ('children' in node) node.children.forEach(collectDefinitions)
        }
        collectDefinitions(tree)
        const walk = (parent: Root | Extract<RootContent, { children: unknown }>, source: string) => {
            parent.children = parent.children.flatMap((node): RootContent | RootContent[] => {
                if (node.type === 'thematicBreak' && blockBreaks.has(node.position!.start.offset!)) return { type: 'break' }
                if (node.type === 'text' && node.value.includes(marker)) {
                    return node.value.split(breakPattern).filter(Boolean).map((value): RootContent =>
                        value.startsWith(marker) ? { type: 'break' } : { type: 'text', value })
                }
                if (node.type === 'link' || node.type === 'image') {
                    node.data = { ...node.data, hProperties: { literalDestination: node.url } }
                }
                if (node.type === 'linkReference' || node.type === 'imageReference') {
                    node.data = { ...node.data, hProperties: { literalDestination: definitions.get(node.identifier) } }
                }
                if (node.type === 'code') {
                    const start = node.position?.start.offset
                    const end = node.position?.end.offset
                    const raw = start !== undefined && end !== undefined ? source.slice(start, end) : ''
                    const opening = raw.match(/^(`{3,}|~{3,})[^\r\n]*(?:\r\n|\n|\r)/)
                    if (opening) {
                        const body = raw.slice(opening[0].length)
                        const fence = opening[1]
                        const closing = body.match(new RegExp(`(^|\\r\\n|\\n|\\r)[ \\t]{0,3}${fence[0]}{${fence.length},}[ \\t]*$`))
                        // The parser removes list/quote prefixes; recover each original line ending.
                        const endings = body.match(/\r\n|\n|\r/g) ?? []
                        const text = node.position?.start.column === 1
                            ? closing?.index === undefined ? body : body.slice(0, closing.index + closing[1].length)
                            : node.value.split('\n').map((line, index) => line + (endings[index] ?? '')).join('')
                        node.data = { ...node.data, hProperties: { copySource: text } }
                    }
                }
                if ('children' in node) walk(node, source)
                return node
            }) as typeof parent.children
        }
        walk(tree, source)
    }
}

function destination(value: string): 'web' | 'anchor' | 'path' | 'unsafe' {
    // Control characters must never reach a navigation or image URL.
    // eslint-disable-next-line no-control-regex
    if (!value || /[\u0000-\u001f\u007f]/.test(value) || value.trim() !== value || value.startsWith('//') || value.startsWith('\\\\')) return 'unsafe'
    if (/^https?:\/\//i.test(value)) {
        try { return new URL(value).hostname ? 'web' : 'unsafe' } catch { return 'unsafe' }
    }
    if (value.startsWith('#')) return 'anchor'
    if (/^[a-z]:[\\/]/i.test(value)) return 'path'
    return /^[a-z][a-z\d+.-]*:/i.test(value) ? 'unsafe' : 'path'
}

function MarkdownLink({ href = '', children, ...props }: ComponentProps<'a'>) {
    const [failed, setFailed] = useState(false)
    const kind = destination(href)
    if (kind === 'web') return <>
        <a {...props} href={href} target="_blank" rel="noopener noreferrer" onClick={(event) => {
            if (isTauri()) {
                event.preventDefault()
                void openUrl(new URL(href).href).then(() => setFailed(false), () => setFailed(true))
            }
        }}>{children}</a>
        {failed ? <span role="status">Could not open link.</span> : null}
    </>
    if (kind === 'anchor') return <a {...props} href={href}>{children}</a>
    return <span>{children}{kind === 'path' ? <TranscriptCopyButton label="Copy path" text={href} /> : null}</span>
}

function MarkdownImage({ src = '', alt = '' }: { src?: string; alt?: string }) {
    const [failedSource, setFailedSource] = useState<string | null>(null)
    const kind = destination(src)
    if (kind === 'web' && failedSource !== src) return <img src={src} alt={alt} loading="lazy" referrerPolicy="no-referrer" onError={() => setFailedSource(src)} />
    return <span>{alt}{kind === 'web' ? <> <MarkdownLink href={src}>{src}</MarkdownLink></> : kind === 'path' ? <TranscriptCopyButton label="Copy path" text={src} /> : null}</span>
}

const markdownComponents: Components = {
    a({ node, ...props }) {
        delete (props as Record<string, unknown>).literalDestination
        return <MarkdownLink {...props} href={typeof node?.properties.literalDestination === 'string' ? node.properties.literalDestination : props.href} />
    },
    img({ node, src, alt }) { return <MarkdownImage src={typeof node?.properties.literalDestination === 'string' ? node.properties.literalDestination : typeof src === 'string' ? src : ''} alt={alt} /> },
    p({ children }) { return <p className="break-words [overflow-wrap:anywhere]">{children}</p> },
    table({ children }) { return <div className="markdown-table"><table>{children}</table></div> },
    pre: function MarkdownPre({ children, node }) {
        const enableCodeCopy = useContext(MarkdownContext)
        const code = node?.children.find((child) => child.type === 'element' && child.tagName === 'code')
        const text = code?.type === 'element' ? code.properties.copySource : undefined
        const mermaid = code?.type === 'element' && Array.isArray(code.properties.className) && code.properties.className.includes('language-mermaid')
        const block = <pre className="overflow-x-auto">{children}</pre>
        return <div className="markdown-code-block">
            {mermaid && typeof text === 'string' ? <MarkdownMermaid source={text}>{block}</MarkdownMermaid> : block}
            {enableCodeCopy && typeof text === 'string' ? <span className="markdown-code-copy"><TranscriptCopyButton label="Copy code" text={text} /></span> : null}
        </div>
    },
    code({ children, className }) { return <code className={className}>{children}</code> },
}

interface ProjectConversationMarkdownProps {
    content: string
    enableCodeCopy?: boolean
}

function ProjectConversationMarkdownComponent({ content, enableCodeCopy = false }: ProjectConversationMarkdownProps) {
    const id = useId()
    const remarkRehypeOptions = useMemo(() => ({ clobberPrefix: `markdown-${id}-`, footnoteLabel: 'Footnotes' }), [id])
    return <MarkdownContext.Provider value={enableCodeCopy}>
        <div className="conversation-markdown min-w-0">
            <ReactMarkdown components={markdownComponents} skipHtml
                remarkPlugins={[remarkGfm, remarkMath, safeBreaksAndCode]}
                rehypePlugins={[[footnoteLabels, `markdown-${id}-`], [rehypeKatex, { trust: false, strict: 'error' }], [rehypeHighlight, { detect: false, ignoreMissing: true }]]}
                remarkRehypeOptions={remarkRehypeOptions}
                urlTransform={(url) => destination(url) === 'unsafe' ? '' : url}
            >{content}</ReactMarkdown>
        </div>
    </MarkdownContext.Provider>
}

export const ProjectConversationMarkdown = memo(ProjectConversationMarkdownComponent)
