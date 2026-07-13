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

  it('shows the unobservable-instructions note only for a selected Codex context capture', () => {
    const codexArtifact = {
      path: 'logs/task/initial-context.txt',
      size_bytes: 12,
      media_type: 'text/plain',
      viewable: true,
      context_capture_kind: 'codex_turn_input' as const,
    }
    const props = {
      artifactDownloadHref: () => null,
      artifactEntries: [codexArtifact],
      artifactError: null,
      artifactViewerError: null,
      artifactViewerPayload: 'Do the task',
      isArtifactViewerLoading: false,
      isLoading: false,
      missingCoreArtifacts: [],
      onRefresh: () => {},
      onViewArtifact: () => {},
      showPartialRunArtifactNote: false,
      status: 'ready' as const,
    }

    const { rerender } = render(<RunArtifactsCard {...props} selectedArtifactEntry={codexArtifact} />)
    expect(screen.getByTestId('run-artifact-codex-context-note')).toBeVisible()

    rerender(
      <RunArtifactsCard
        {...props}
        selectedArtifactEntry={{ ...codexArtifact, context_capture_kind: 'assembled_messages' }}
      />,
    )
    expect(screen.queryByTestId('run-artifact-codex-context-note')).not.toBeInTheDocument()
  })

})
