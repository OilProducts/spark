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
