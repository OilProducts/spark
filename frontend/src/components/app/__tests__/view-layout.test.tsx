import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'

import { ViewLayout } from '@/components/app/view-layout'

it('hides the side panel to a show button and back, keeping the panel mounted and focus on the toggle', () => {
  render(
    <ViewLayout view="runs" title="Runs" panel={<input aria-label="Search runs" defaultValue="audit" />}>
      <p>Run detail</p>
    </ViewLayout>,
  )
  const panel = screen.getByRole('complementary', { name: 'Runs panel' })

  fireEvent.click(screen.getByRole('button', { name: 'Hide Runs panel' }))
  expect(panel).toHaveAttribute('data-hidden', 'true')
  expect(screen.queryByText('Runs')).not.toBeInTheDocument()
  // Hidden, not unmounted: what was typed in the panel is still there when it comes back.
  expect(screen.getByLabelText('Search runs', { selector: 'input' })).not.toBeVisible()
  const show = screen.getByRole('button', { name: 'Show Runs panel' })
  expect(show).toHaveFocus()

  fireEvent.click(show)
  expect(panel).not.toHaveAttribute('data-hidden')
  expect(screen.getByLabelText('Search runs')).toHaveValue('audit')
  expect(screen.getByRole('button', { name: 'Hide Runs panel' })).toHaveFocus()
  expect(screen.getByText('Run detail')).toBeVisible()
})
