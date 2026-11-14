import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { RunArtifactViewer } from '../components/RunArtifactViewer'

const view = (path: string, payload: string) => render(
    <RunArtifactViewer entry={{ path }} open onOpenChange={() => {}} isLoading={false} error={null} payload={payload} downloadHref={null} />,
)

it('shows a Markdown file formatted, with its source a click away, and other files as text', () => {
    const { unmount } = view('result.md', '# Done\n\nAll **green**.')
    expect(screen.getByRole('heading', { name: 'Done' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show source' }))
    expect(screen.getByTestId('run-artifact-viewer-payload').tagName).toBe('PRE')
    expect(screen.getByTestId('run-artifact-viewer-payload')).toHaveTextContent('# Done All **green**.')
    unmount()

    view('status.json', '{"outcome": "success"}')
    expect(screen.getByTestId('run-artifact-viewer-payload').tagName).toBe('PRE')
    expect(screen.queryByRole('button', { name: 'Show source' })).not.toBeInTheDocument()
})

it('keeps the lines of plain-text output in a Markdown file', () => {
    view('result.md', 'collected 2 items\ntests/test_a.py ..\n2 passed in 0.01s')
    const payload = screen.getByTestId('run-artifact-viewer-payload')
    expect(payload.querySelectorAll('br')).toHaveLength(2)
    expect(payload.querySelector('p')?.innerHTML).toMatch(/^collected 2 items<br>\ntests\/test_a\.py \.\.<br>\n2 passed in 0\.01s$/)
})

it('offers Try again after a file fails to load, where loading it again can help', () => {
    let retries = 0
    const { rerender } = render(
        <RunArtifactViewer entry={{ path: 'result.md' }} open onOpenChange={() => {}} isLoading={false} error="Spark is unavailable." onRetry={() => { retries += 1 }} payload={null} downloadHref={null} />,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('Spark is unavailable.')
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(retries).toBe(1)
    rerender(<RunArtifactViewer entry={{ path: 'image.png' }} open onOpenChange={() => {}} isLoading={false} error="This kind of file can’t be shown here." payload={null} downloadHref={null} />)
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument()
})
