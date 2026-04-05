import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { RunArtifactsCard } from '@/features/runs/components/RunArtifactsCard'
import { RunCheckpointCard } from '@/features/runs/components/RunCheckpointCard'
import { RunContextCard } from '@/features/runs/components/RunContextCard'

describe('Run detail status cards', () => {
  it('renders restoring states instead of false empty states before context is ready', () => {
    render(
      <RunContextCard
        collapsed={false}
        contextCopyStatus=""
        contextError={null}
        contextExportHref={null}
        filteredContextRows={[]}
        isLoading={true}
        onCollapsedChange={() => {}}
        onCopy={() => {}}
        onRefresh={() => {}}
        onSearchQueryChange={() => {}}
        runId="run-1"
        searchQuery=""
        status="loading"
      />,
    )

    expect(screen.getByTestId('run-context-loading')).toBeVisible()
    expect(screen.queryByTestId('run-context-empty')).not.toBeInTheDocument()
  })

  it('renders restoring states instead of false empty states before artifacts are ready', () => {
    render(
      <RunArtifactsCard
        artifactDownloadHref={() => null}
        artifactEntries={[]}
        artifactError={null}
        artifactViewerError={null}
        artifactViewerPayload={null}
        collapsed={false}
        isArtifactViewerLoading={false}
        isLoading={true}
        missingCoreArtifacts={[]}
        onCollapsedChange={() => {}}
        onRefresh={() => {}}
        onViewArtifact={() => {}}
        selectedArtifactEntry={null}
        showPartialRunArtifactNote={false}
        status="loading"
      />,
    )

    expect(screen.getByTestId('run-artifact-loading')).toBeVisible()
    expect(screen.queryByTestId('run-artifact-empty')).not.toBeInTheDocument()
  })

  it('distinguishes checkpoint restoring from checkpoint ready-empty', () => {
    const { rerender } = render(
      <RunCheckpointCard
        checkpointCompletedNodes="—"
        checkpointCurrentNode="—"
        checkpointData={null}
        checkpointError={null}
        checkpointRetryCounters="—"
        collapsed={false}
        isLoading={true}
        onCollapsedChange={() => {}}
        onRefresh={() => {}}
        status="loading"
      />,
    )

    expect(screen.getByTestId('run-checkpoint-loading')).toBeVisible()
    expect(screen.queryByTestId('run-checkpoint-empty')).not.toBeInTheDocument()

    rerender(
      <RunCheckpointCard
        checkpointCompletedNodes="—"
        checkpointCurrentNode="—"
        checkpointData={null}
        checkpointError={null}
        checkpointRetryCounters="—"
        collapsed={false}
        isLoading={false}
        onCollapsedChange={() => {}}
        onRefresh={() => {}}
        status="ready"
      />,
    )

    expect(screen.getByTestId('run-checkpoint-empty')).toBeVisible()
  })
})
