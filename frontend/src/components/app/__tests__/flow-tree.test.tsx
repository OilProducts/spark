import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { FlowTree } from '@/components/app/flow-tree'

describe('FlowTree', () => {
  it('renders directories open by default, collapses folders, and selects the requested flow', async () => {
    const user = userEvent.setup()
    const onSelectFlow = vi.fn()

    render(
      <FlowTree
        dataTestId="shared-flow-tree"
        flows={['root.dot', 'team/review/nested.dot']}
        selectedFlow="root.dot"
        onSelectFlow={onSelectFlow}
        renderFlowIndicator={(flowName) =>
          flowName === 'team/review/nested.dot' ? <span data-testid="flow-indicator">active</span> : null
        }
      />,
    )

    const flowTree = screen.getByTestId('shared-flow-tree')
    expect(within(flowTree).getByText('team')).toBeVisible()
    expect(within(flowTree).getByText('review')).toBeVisible()
    expect(within(flowTree).getByTestId('flow-indicator')).toBeVisible()

    await user.click(within(flowTree).getByRole('button', { name: 'team' }))
    expect(within(flowTree).queryByRole('button', { name: 'team/review/nested.dot' })).not.toBeInTheDocument()

    await user.click(within(flowTree).getByRole('button', { name: 'team' }))
    expect(within(flowTree).getByText('review')).toBeVisible()

    await user.click(within(flowTree).getByRole('button', { name: 'team/review/nested.dot' }))
    expect(onSelectFlow).toHaveBeenCalledWith('team/review/nested.dot')
  })

  it('lists the project flows first under their own heading and the rest under Bundled', async () => {
    const user = userEvent.setup()
    const onSelectFlow = vi.fn()
    const flows = ['examples/simple-linear', 'nightly-audit', 'team/release']

    const { rerender } = render(
      <FlowTree flows={flows} projectFlows={new Set(['nightly-audit', 'team/release'])} selectedFlow={null} onSelectFlow={onSelectFlow} />,
    )

    expect(screen.getAllByRole('heading', { level: 3 }).map((heading) => heading.textContent)).toEqual(['This project', 'Bundled'])
    const project = screen.getByRole('region', { name: 'This project' })
    expect(within(project).getByRole('button', { name: 'nightly-audit' })).toBeVisible()
    expect(within(project).getByText('team')).toBeVisible()
    expect(within(screen.getByRole('region', { name: 'Bundled' })).getByText('examples')).toBeVisible()
    await user.click(within(project).getByRole('button', { name: 'team/release' }))
    expect(onSelectFlow).toHaveBeenCalledWith('team/release')

    // Without project flows (or before they are known) the tree is one unlabelled list.
    rerender(<FlowTree flows={flows} projectFlows={new Set()} selectedFlow={null} onSelectFlow={onSelectFlow} />)
    expect(screen.queryByRole('heading')).toBeNull()
    expect(screen.getByRole('button', { name: 'nightly-audit' })).toBeVisible()
  })

  it('routes delete actions without selecting the flow', async () => {
    const user = userEvent.setup()
    const onSelectFlow = vi.fn()
    const onDeleteFlow = vi.fn()

    render(
      <FlowTree
        flows={['team/review/nested.dot']}
        selectedFlow={null}
        onSelectFlow={onSelectFlow}
        onDeleteFlow={onDeleteFlow}
      />,
    )

    await user.click(screen.getByTitle('Delete team/review/nested.dot'))

    expect(onDeleteFlow).toHaveBeenCalledWith(expect.any(Object), 'team/review/nested.dot')
    expect(onSelectFlow).not.toHaveBeenCalled()
  })
})
