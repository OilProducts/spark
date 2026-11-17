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
    expect(within(flowTree).getByText('Team')).toBeVisible()
    expect(within(flowTree).getByText('Review')).toBeVisible()
    expect(within(flowTree).getByTestId('flow-indicator')).toBeVisible()

    await user.click(within(flowTree).getByRole('button', { name: 'Team' }))
    expect(within(flowTree).queryByRole('button', { name: 'team/review/nested.dot' })).not.toBeInTheDocument()

    await user.click(within(flowTree).getByRole('button', { name: 'Team' }))
    expect(within(flowTree).getByText('Review')).toBeVisible()

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

    expect(screen.getAllByRole('heading', { level: 3 }).map((heading) => heading.textContent)).toEqual(['This project', 'Bundled with Spark'])
    const project = screen.getByRole('region', { name: 'This project' })
    expect(within(project).getByRole('button', { name: 'nightly-audit' })).toBeVisible()
    expect(within(project).getByText('Team')).toBeVisible()
    expect(within(screen.getByRole('region', { name: 'Bundled with Spark' })).getByText('Examples')).toBeVisible()
    await user.click(within(project).getByRole('button', { name: 'team/release' }))
    expect(onSelectFlow).toHaveBeenCalledWith('team/release')

    // Named for its project when given; a flow with a title shows it, its description under it.
    rerender(<FlowTree flows={flows} projectFlows={new Set(['nightly-audit'])} projectLabel="acme" flowLabels={{ 'nightly-audit': { title: 'Nightly audit', description: 'Checks the build every night.' } }} selectedFlow={null} onSelectFlow={onSelectFlow} />)
    const named = screen.getByRole('region', { name: "acme's flows" })
    expect(within(named).getByRole('button', { name: 'Nightly audit (nightly-audit)' })).toHaveTextContent('Checks the build every night.')
    // The description is the row's accessible description, so its name matches the visible title.
    expect(within(named).getByRole('button', { name: 'Nightly audit (nightly-audit)' })).toHaveAccessibleDescription('Checks the build every night.')

    // A search that leaves none of the project's flows keeps the rest under their heading.
    rerender(<FlowTree flows={['examples/simple-linear']} projectFlows={new Set(['nightly-audit'])} selectedFlow={null} onSelectFlow={onSelectFlow} />)
    expect(screen.getAllByRole('heading', { level: 3 }).map((heading) => heading.textContent)).toEqual(['Bundled with Spark'])

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
