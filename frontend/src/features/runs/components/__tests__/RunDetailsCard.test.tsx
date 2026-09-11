import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { RunDetailsCard } from '../RunDetailsCard'

describe('execution lock details', () => {
    it('labels inherited protection without claiming lock ownership', () => {
        render(<RunDetailsCard activeProjectPath={null} now={0} run={{
            run_id: 'child', parent_run_id: 'parent', flow_name: 'child',
            status: 'running', working_directory: '/tmp/project', model: '', started_at: '',
            execution_lock: {
                scope: 'project', key: 'integration', conflict_policy: 'queue',
                identity: 'project:integration', state: 'inherited',
            },
        }} />)
        expect(screen.getByTestId('run-summary-execution-lock-state')).toHaveTextContent('Inherited from parent')
        expect(screen.queryByText('Holding execution lock')).not.toBeInTheDocument()
    })
})
