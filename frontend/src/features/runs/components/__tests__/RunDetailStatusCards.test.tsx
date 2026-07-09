import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { RunArtifactsCard } from '@/features/runs/components/RunArtifactsCard'
import { RunContextCard } from '@/features/runs/components/RunContextCard'

describe('Run detail status cards', () => {
  it('renders restoring states instead of false empty states before context is ready', () => {
    render(
      <RunContextCard
        contextCopyStatus=""
        contextError={null}
        contextExportHref={null}
        filteredContextRows={[]}
        isLoading={true}
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
        isArtifactViewerLoading={false}
        isLoading={true}
        missingCoreArtifacts={[]}
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

})
