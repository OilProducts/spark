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
