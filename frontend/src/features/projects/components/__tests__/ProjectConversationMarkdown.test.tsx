import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { StrictMode, useState } from 'react'
import { ProjectConversationMarkdown } from '../ProjectConversationMarkdown'
import { RunTranscriptRowItem, useTranscriptExpansion } from '@/features/runs/components/RunTranscriptGroups'
import { buildRunTranscriptRow } from '@/features/runs/model/transcriptModel'
import { MessageRow, ThinkingRow, ToolCallRow } from '@/components/app/transcript/SegmentRows'

const { renderDiagram, initialize, isTauri, openUrl } = vi.hoisted(() => ({
    renderDiagram: vi.fn(), initialize: vi.fn(), isTauri: vi.fn(() => false), openUrl: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('mermaid', () => ({ default: { initialize, render: renderDiagram } }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }))
afterEach(() => { cleanup(); vi.clearAllMocks(); vi.unstubAllGlobals(); isTauri.mockReturnValue(false) })
const markdown = (content: string, enableCodeCopy = true) => <ProjectConversationMarkdown content={content} enableCodeCopy={enableCodeCopy} />

describe('shared Markdown', () => {
    it('renders GFM tables, alignment, read-only tasks, deletion and ordered starts', () => {
        const { container } = render(markdown('| Left | Right |\n| :--- | ---: |\n| a | b |\n\n- [x] done\n- [ ] todo\n\n~~removed~~\n\n7. seven\n8. eight'))
        expect(screen.getAllByRole('columnheader')).toHaveLength(2)
        expect(screen.getAllByRole('columnheader')[1]).toHaveStyle({ textAlign: 'right' })
        expect(screen.getAllByRole('checkbox').every((input) => (input as HTMLInputElement).disabled)).toBe(true)
        expect(container.querySelector('del')).toHaveTextContent('removed')
        expect(container.querySelector('ol')).toHaveAttribute('start', '7')
    })

    it('copies exact fences, highlights known languages, and keeps nested/unknown code plain', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        vi.stubGlobal('navigator', { clipboard: { writeText } })
        const { container } = render(markdown('```js meta\r\n  const x = 1;  \r\n\r\n```\r\n\r\n- nested\n\n  ```unknown-lang\n    code <br>\n  ```'))
        expect(container.querySelector('.hljs-keyword')).toHaveTextContent('const')
        expect(container.querySelector('code.language-unknown-lang')).toHaveTextContent('code <br>')
        const buttons = screen.getAllByRole('button', { name: 'Copy code' })
        fireEvent.click(buttons[0])
        await waitFor(() => expect(writeText).toHaveBeenLastCalledWith('  const x = 1;  \r\n\r\n'))
        fireEvent.click(buttons[1])
        await waitFor(() => expect(writeText).toHaveBeenLastCalledWith('  code <br>\n'))
    })

    it('copies quoted fences without Markdown container prefixes and retains CRLF', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        vi.stubGlobal('navigator', { clipboard: { writeText } })
        render(markdown('> ~~~text\r\n>   indented  \n> second\r\n> ~~~'))
        fireEvent.click(screen.getByRole('button', { name: 'Copy code' }))
        await waitFor(() => expect(writeText).toHaveBeenCalledWith('  indented  \nsecond\r\n'))
    })

    it('handles external links, literal paths, unsafe schemes and desktop errors', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        vi.stubGlobal('navigator', { clipboard: { writeText } })
        render(markdown('[web](https://example.com/a) [file](/tmp/a.ts:12) [relative](../a.ts:9) [spaces](<./a b.ts:4>) [bad](javascript:alert%281%29) [data](data:text/html,bad) [file-url](file:///tmp/a) [protocol](//evil.test) [reference][path]\n\n[path]: <./literal path.ts:8>'))
        const link = screen.getByRole('link', { name: 'web' })
        expect(link).toHaveAttribute('target', '_blank')
        expect(link).toHaveAttribute('rel', 'noopener noreferrer')
        expect(screen.getAllByRole('link')).toHaveLength(1)
        for (const [index, path] of ['/tmp/a.ts:12', '../a.ts:9', './a b.ts:4', './literal path.ts:8'].entries()) {
            fireEvent.click(screen.getAllByRole('button', { name: /^Copy path/ })[index])
            await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(path))
        }
        isTauri.mockReturnValue(true)
        fireEvent.click(link)
        await waitFor(() => expect(openUrl).toHaveBeenCalledWith('https://example.com/a'))
        openUrl.mockRejectedValueOnce(new Error('denied'))
        fireEvent.click(link)
        expect(await screen.findByRole('status')).toHaveTextContent('Could not open link')
    })

    it.each([
        ['HTTP://EXAMPLE.COM:80/Path?Key=Value#Section', 'http://example.com/Path?Key=Value#Section'],
        ['HTTPS://EXAMPLE.COM:443/Path?Key=Value#Section', 'https://example.com/Path?Key=Value#Section'],
        ['hTtP://Example.Com/Path?Key=Value#Section', 'http://example.com/Path?Key=Value#Section'],
        ['hTtPs://Example.Com/Path?Key=Value#Section', 'https://example.com/Path?Key=Value#Section'],
    ])('normalizes desktop destination %s through the actual renderer', async (source, normalized) => {
        isTauri.mockReturnValue(true)
        render(markdown(`[web](${source})`))
        fireEvent.click(screen.getByRole('link', { name: 'web' }))
        await waitFor(() => expect(openUrl).toHaveBeenCalledExactlyOnceWith(normalized))
    })

    it('keeps every footnote target and accessibility label local to its renderer', () => {
        const source = 'Note[^1].\n\n[^1]: Footnote'
        const { container } = render(<>{markdown(source)}{markdown(source)}</>)
        const ids = [...container.querySelectorAll('[id]')].map((element) => element.id)
        expect(new Set(ids).size).toBe(ids.length)
        for (const renderer of container.querySelectorAll('.conversation-markdown')) {
            for (const link of renderer.querySelectorAll('a')) {
                const target = link.getAttribute('href')!.slice(1)
                expect([...renderer.querySelectorAll('[id]')].some((element) => element.id === target)).toBe(true)
                const label = link.getAttribute('aria-describedby')
                if (label) expect([...renderer.querySelectorAll('[id]')].some((element) => element.id === label)).toBe(true)
            }
        }
    })

    it('uses the shared renderer through the Run transcript adapter', () => {
        const row = buildRunTranscriptRow({
            id: 'run', turn_id: 'turn', order: 1, kind: 'assistant_message', role: 'assistant',
            status: 'complete', timestamp: '', updated_at: '', content: '~~Run output~~ $x$',
            node_id: 'node', attempt: 1, source_scope: 'root', source_parent_node_id: null,
            source_flow_name: null, source_run_id: null, latest_sequence: 1,
        })!
        function Run() { return <RunTranscriptRowItem row={row} expansion={useTranscriptExpansion()} /> }
        const { container } = render(<Run />)
        expect(screen.getByText('Run output').tagName).toBe('DEL')
        expect(container.querySelector('.katex')).toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'Copy message' })).not.toBeInTheDocument()
    })

    it('loads safe images, falls back after failure, and offers local image path copying', () => {
        render(markdown('![remote](https://example.com/a.png) ![local](./a.png) ![unsafe](data:image/svg+xml,bad)'))
        const image = screen.getByRole('img', { name: 'remote' })
        expect(image).toHaveAttribute('loading', 'lazy')
        expect(image).toHaveAttribute('referrerpolicy', 'no-referrer')
        expect(screen.getAllByRole('img')).toHaveLength(1)
        fireEvent.error(image)
        expect(screen.getByText('remote')).toBeVisible()
        expect(screen.getByRole('link')).toHaveAttribute('href', 'https://example.com/a.png')
        expect(screen.getByRole('button', { name: 'Copy path' })).toBeVisible()
    })

    it.each(['<br>', '<br/>', '<br />'])('preserves text after standalone %s HTML nodes', (tag) => {
        const { container } = render(markdown(`${tag}\nVisible after break`))
        expect(container.querySelectorAll('br')).toHaveLength(1)
        expect(screen.getByText('Visible after break')).toBeVisible()
    })

    it.each(['<br>', '<br/>', '<br />'])('preserves consecutive standalone %s breaks', (tag) => {
        const { container } = render(markdown(`${tag}\n${tag}\nVisible after break`))
        expect(container.querySelectorAll('br')).toHaveLength(2)
        expect(screen.getByText('Visible after break')).toBeVisible()
    })

    it.each(['<br>', '<br/>', '<br />'])('resolves document references after %s, including literal paths', async (tag) => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        vi.stubGlobal('navigator', { clipboard: { writeText } })
        const { container } = render(markdown(`${tag}\n[docs][ref] ![remote][image] [file][path] ![local][local-image]\n\n[ref]: https://example.com\n[image]: https://example.com/a.png\n[path]: <./literal path.ts:8>\n[local-image]: <../local image.png>`))
        expect(container.querySelectorAll('br')).toHaveLength(1)
        expect(screen.getByRole('link', { name: 'docs' })).toHaveAttribute('href', 'https://example.com')
        expect(screen.getByRole('img', { name: 'remote' })).toHaveAttribute('src', 'https://example.com/a.png')
        for (const [index, path] of ['./literal path.ts:8', '../local image.png'].entries()) {
            fireEvent.click(screen.getAllByRole('button', { name: /^Copy path/ })[index])
            await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(path))
        }
    })

    it.each(['<br>', '<br/>', '<br />'])('keeps recovered %s footnotes instance-local with definitions outside the fragment', (tag) => {
        const source = `${tag}\nNote[^1].\n\n[^1]: Footnote with [docs][ref].\n\n[ref]: https://example.com`
        const { container } = render(<>{markdown(source)}{markdown(source)}</>)
        expect(screen.getAllByRole('link', { name: 'docs' })).toHaveLength(2)
        expect(container.querySelectorAll('[data-footnote-ref]')).toHaveLength(2)
        const ids = [...container.querySelectorAll('[id]')].map((element) => element.id)
        expect(new Set(ids).size).toBe(ids.length)
        for (const renderer of container.querySelectorAll('.conversation-markdown')) {
            expect(renderer.querySelector('[data-footnotes]')).toHaveTextContent('Footnote with docs.')
            for (const link of renderer.querySelectorAll('a[href^="#"]')) {
                const localIds = [...renderer.querySelectorAll('[id]')].map((element) => element.id)
                expect(localIds).toContain(link.getAttribute('href')!.slice(1))
                if (link.hasAttribute('aria-describedby')) expect(localIds).toContain(link.getAttribute('aria-describedby'))
            }
        }
    })

    it('resolves definitions recovered in another HTML fragment and preserves following code offsets', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        vi.stubGlobal('navigator', { clipboard: { writeText } })
        const { container } = render(markdown('[docs][ref] Note[^1].\r\n\r\n<br>\r\n[ref]: https://example.com\r\n\r\n<br/>\r\n[^1]: Footnote\r\n\r\n```text\r\n  <br />  \r\n\r\n```'))
        expect(screen.getByRole('link', { name: 'docs' })).toHaveAttribute('href', 'https://example.com')
        expect(container.querySelector('[data-footnotes]')).toHaveTextContent('Footnote')
        fireEvent.click(screen.getByRole('button', { name: 'Copy code' }))
        await waitFor(() => expect(writeText).toHaveBeenCalledWith('  <br />  \r\n\r\n'))
    })

    it('preserves breaks inside emphasis and leaves literal placeholder-like text alone', () => {
        const { container } = render(markdown('*a<br>b* **c<br/>d** [e<br />f](https://example.com) &#xE000;bb&#57344; \uE001bb\uE001'))
        expect(container.querySelectorAll('br')).toHaveLength(3)
        expect(container.querySelector('em')).toHaveTextContent('a b')
        expect(container.querySelector('strong')).toHaveTextContent('c d')
        expect(screen.getByRole('link', { name: /e\s+f/ })).toHaveAttribute('href', 'https://example.com')
        expect(container.textContent).toContain('\uE000bb\uE000 \uE001bb\uE001')
    })

    it('recovers Markdown after breaks while suppressing unsafe HTML and preserving code and soft breaks', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined)
        Object.assign(navigator, { clipboard: { writeText } })
        const { container } = render(markdown('<br>\nVisible <b>after</b> break\nsoft\n<br onclick="evil()">\n\n<script>evil()</script>\n\n`<br>`\n\n```text\n<br>\n<br/>\n<br />\n```'))
        expect(container.querySelectorAll('br')).toHaveLength(1)
        expect(container.textContent).toContain('Visible after break\nsoft')
        expect(container.querySelector('script, b, [onclick]')).toBeNull()
        expect(container.textContent).not.toContain('evil()')
        expect(container.querySelector('p code')?.textContent).toBe('<br>')
        expect(container.querySelector('pre code')?.textContent).toBe('<br>\n<br/>\n<br />\n')
        fireEvent.click(screen.getByRole('button', { name: 'Copy code' }))
        await waitFor(() => expect(writeText).toHaveBeenCalledWith('<br>\n<br/>\n<br />\n'))
    })

    it('renders math safely and only transforms attribute-free HTML breaks outside code', () => {
        const { container } = render(markdown('a<br>b<br/>c<br />d\nsoft\n\n`<br>`\n\n```html\n<br>\n```\n\n<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\nx<br onclick="evil()">y\n\n$x^2$\n\n$$\n\\frac{1}{2}\n$$\n\n$\\notACommand{x}$\n\n$\\href{javascript:alert(1)}{unsafe}$'))
        expect(container.querySelectorAll('br')).toHaveLength(3)
        expect(container.querySelectorAll('.katex').length).toBeGreaterThanOrEqual(2)
        expect(container.textContent).toContain('\\notACommand')
        expect(container.querySelector('script, img, [onclick], a[href^="javascript"]')).toBeNull()
        expect(container.querySelector('p')?.textContent).toContain('d\nsoft')
    })

    it.each(['project', 'run'])('renders expanded %s thinking with optional bold headings, leaving raw output literal', (prefix) => {
        function Thinking({ content }: { content: string }) {
            const [expanded, setExpanded] = useState(false)
            return <ThinkingRow entry={{ id: 'thinking', role: 'assistant', status: 'streaming', timestamp: '', content }} isExpanded={expanded} onToggleThinkingEntryExpanded={() => setExpanded(!expanded)} formatConversationTimestamp={() => ''} testIdPrefix={prefix} />
        }
        const { rerender } = render(<Thinking content={'**Reasoning**\n\n~~body~~'} />)
        expect(screen.queryByText('body')).not.toBeInTheDocument()
        fireEvent.click(screen.getByRole('button', { name: 'Reasoning' }))
        expect(screen.getByText('body').tagName).toBe('DEL')
        rerender(<Thinking content={'## Full body\n\n$x$'} />)
        expect(screen.getByRole('button', { name: 'Thinking' })).toBeVisible()
        expect(screen.getByRole('heading', { name: 'Full body' })).toBeVisible()
        expect(screen.queryByRole('button', { name: 'Copy code' })).not.toBeInTheDocument()
        rerender(<MessageRow entry={{ id: 'user', role: 'user', content: '**literal**', status: 'complete', timestamp: '' }} formatConversationTimestamp={() => ''} />)
        expect(screen.getByText('**literal**')).toBeVisible()
        rerender(<ToolCallRow entry={{ id: 'tool', timestamp: '', toolCall: { id: 'tool', kind: 'dynamic_tool', status: 'completed', title: 'Tool', output: '**raw**', filePaths: [] } }} isExpanded onToggleToolCallExpanded={() => {}} />)
        expect(screen.getByText('**raw**', { selector: 'pre' })).toBeVisible()
    })
})

const fence = (source: string) => '```mermaid\n' + source
const deferred = () => {
    let resolve!: (value: { svg: string }) => void
    let reject!: (reason: Error) => void
    const promise = new Promise<{ svg: string }>((yes, no) => { resolve = yes; reject = no })
    return { promise, resolve, reject }
}

describe('Mermaid lifecycle', () => {
    it('renders invalid to valid streaming source and keeps source/copy available', async () => {
        renderDiagram.mockRejectedValueOnce(new Error('incomplete')).mockResolvedValueOnce({ svg: '<svg><text>valid</text></svg>' })
        const { rerender } = render(markdown(fence('graph'), false))
        await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(1))
        expect(screen.queryByRole('img')).not.toBeInTheDocument()
        expect(screen.getByText('graph')).toBeVisible()
        rerender(markdown(fence('graph TD; A-->B'), true))
        expect(await screen.findByRole('img', { name: 'Mermaid diagram' })).toBeVisible()
        expect(screen.getByText('Diagram source')).toBeVisible()
        expect(screen.getByRole('button', { name: 'Copy code' })).toBeVisible()
        expect(initialize).toHaveBeenCalledWith({ startOnLoad: false, securityLevel: 'strict', suppressErrorRendering: true })
    })

    it('coalesces rapid updates, ignores stale success/failure and does not overlap renders', async () => {
        const first = deferred(), latest = deferred()
        renderDiagram.mockReturnValueOnce(first.promise).mockReturnValueOnce(latest.promise)
        const { rerender } = render(markdown(fence('A')))
        await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(1))
        rerender(markdown(fence('B')))
        rerender(markdown(fence('C')))
        expect(renderDiagram).toHaveBeenCalledTimes(1)
        await act(async () => first.resolve({ svg: '<svg><text>stale</text></svg>' }))
        expect(renderDiagram).toHaveBeenCalledTimes(2)
        expect(renderDiagram.mock.calls[1][1]).toBe('C')
        expect(screen.queryByRole('img')).not.toBeInTheDocument()
        await act(async () => latest.resolve({ svg: '<svg><text>latest</text></svg>' }))
        expect(screen.getByRole('img')).toHaveTextContent('latest')
        const staleFailure = deferred()
        renderDiagram.mockReturnValueOnce(staleFailure.promise).mockResolvedValueOnce({ svg: '<svg><text>new</text></svg>' })
        rerender(markdown(fence('D')))
        await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(3))
        expect(screen.queryByRole('img')).not.toBeInTheDocument()
        rerender(markdown(fence('E')))
        await act(async () => staleFailure.reject(new Error('old failure')))
        expect(await screen.findByRole('img')).toHaveTextContent('new')
    })

    it('isolates multiple diagrams and ignores completions and pending updates after unmount in StrictMode', async () => {
        const first = deferred(), second = deferred()
        renderDiagram.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
        const { rerender, unmount } = render(<StrictMode>{markdown('```mermaid\nA\n```\n\n```mermaid\nB\n```')}</StrictMode>)
        await waitFor(() => expect(renderDiagram).toHaveBeenCalledTimes(2))
        expect(renderDiagram.mock.calls[0][0]).not.toBe(renderDiagram.mock.calls[1][0])
        rerender(<StrictMode>{markdown('```mermaid\nC\n```\n\n```mermaid\nD\n```')}</StrictMode>)
        unmount()
        await act(async () => { first.resolve({ svg: '<svg />' }); second.reject(new Error('late')) })
        expect(renderDiagram).toHaveBeenCalledTimes(2)
        expect(screen.queryByRole('img')).not.toBeInTheDocument()
    })
})
