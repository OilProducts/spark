import ReactMarkdown, { type Components } from 'react-markdown'

import { cn } from '@/lib/utils'

const INLINE_CODE_CONTAINER_CLASS_NAME =
    '[&_code]:rounded [&_code]:border [&_code]:border-border/60 [&_code]:bg-background/80 [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[11px] [&_code]:text-foreground'

const markdownComponents: Components = {
    a({ children }) {
        return <span className="font-medium text-foreground">{children}</span>
    },
    blockquote({ children }) {
        return (
            <blockquote
                className={cn(
                    'border-l-2 border-border/80 pl-3 italic text-muted-foreground',
                    INLINE_CODE_CONTAINER_CLASS_NAME,
                )}
            >
                {children}
            </blockquote>
        )
    },
    code({ children, className }) {
        return <code className={cn('font-mono text-[11px] text-foreground', className)}>{children}</code>
    },
    em({ children }) {
        return <em className="italic text-foreground">{children}</em>
    },
    h1({ children }) {
        return (
            <h1 className={cn('text-sm font-semibold text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h1>
        )
    },
    h2({ children }) {
        return (
            <h2 className={cn('text-sm font-semibold text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h2>
        )
    },
    h3({ children }) {
        return (
            <h3 className={cn('text-xs font-semibold uppercase tracking-wide text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h3>
        )
    },
    h4({ children }) {
        return (
            <h4 className={cn('text-xs font-semibold text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h4>
        )
    },
    h5({ children }) {
        return (
            <h5 className={cn('text-xs font-semibold text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h5>
        )
    },
    h6({ children }) {
        return (
            <h6 className={cn('text-xs font-semibold text-muted-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </h6>
        )
    },
    img({ alt }) {
        if (!alt) {
            return null
        }

        return <span className="font-medium text-foreground">{alt}</span>
    },
    li({ children }) {
        return <li className={cn('break-words', INLINE_CODE_CONTAINER_CLASS_NAME)}>{children}</li>
    },
    ol({ children }) {
        return <ol className="list-decimal space-y-1 pl-5 text-xs leading-5 text-foreground">{children}</ol>
    },
    p({ children }) {
        return (
            <p className={cn('min-w-0 break-words text-xs leading-5 text-foreground', INLINE_CODE_CONTAINER_CLASS_NAME)}>
                {children}
            </p>
        )
    },
    pre({ children }) {
        return (
            <pre className="overflow-x-auto rounded border border-border/60 bg-background/80 px-3 py-2">
                {children}
            </pre>
        )
    },
    strong({ children }) {
        return <strong className="font-semibold text-foreground">{children}</strong>
    },
    ul({ children }) {
        return <ul className="list-disc space-y-1 pl-5 text-xs leading-5 text-foreground">{children}</ul>
    },
}

interface ProjectConversationMarkdownProps {
    content: string
}

export function ProjectConversationMarkdown({ content }: ProjectConversationMarkdownProps) {
    return (
        <div className="space-y-2 text-foreground">
            <ReactMarkdown components={markdownComponents} skipHtml>
                {content}
            </ReactMarkdown>
        </div>
    )
}
