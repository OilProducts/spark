import { useState } from 'react'

import type { ContextErrorState, RunContextRow } from '../model/shared'
import {
    Button,
    InlineNotice,
    Input,
    Panel,
    PanelContent,
    PanelHeader,
    SectionHeader,
} from '@/ui'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunContextCardProps {
    isLoading: boolean
    contextError: ContextErrorState | null
    contextCopyStatus: string
    searchQuery: string
    filteredContextRows: RunContextRow[]
    contextExportHref: string | null
    runId: string
    onRefresh: () => void | Promise<void>
    onCopy: () => void | Promise<void>
    onSearchQueryChange: (value: string) => void
}

export function RunContextCard({
    isLoading,
    contextError,
    contextCopyStatus,
    searchQuery,
    filteredContextRows,
    contextExportHref,
    runId,
    onRefresh,
    onCopy,
    onSearchQueryChange,
}: RunContextCardProps) {
    const [collapsed, setCollapsed] = useState(false)
    const exportButton = contextExportHref ? (
        <Button
            asChild
            variant="outline"
            size="xs"
            className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
        >
            <a
                data-testid="run-context-export-button"
                href={contextExportHref}
                download={`run-context-${runId}.json`}
            >
                Export JSON
            </a>
        </Button>
    ) : (
        <Button
            type="button"
            data-testid="run-context-export-button"
            disabled={true}
            variant="outline"
            size="xs"
            className="h-7 text-[11px] text-muted-foreground"
        >
            Export JSON
        </Button>
    )

    return (
        <Panel data-testid="run-context-panel">
            <PanelHeader>
                <SectionHeader
                    title="Context"
                    description="Search, copy, or export the structured runtime context."
                    action={(
                        <div className="flex items-center gap-2">
                            <Button
                                onClick={onRefresh}
                                data-testid="run-context-refresh-button"
                                variant="outline"
                                size="xs"
                                className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                            >
                                {isLoading ? 'Refreshing…' : 'Refresh'}
                            </Button>
                            <Button
                                type="button"
                                onClick={onCopy}
                                data-testid="run-context-copy-button"
                                variant="outline"
                                size="xs"
                                className="h-7 text-[11px] text-muted-foreground hover:text-foreground"
                            >
                                Copy JSON
                            </Button>
                            {exportButton}
                            <RunSectionToggleButton
                                collapsed={collapsed}
                                onToggle={() => setCollapsed((current) => !current)}
                                testId="run-context-toggle-button"
                            />
                        </div>
                    )}
                />
            </PanelHeader>
            {!collapsed ? (
                <PanelContent className="space-y-3">
            {contextCopyStatus && (
                <div data-testid="run-context-copy-status" className="text-xs text-muted-foreground">
                    {contextCopyStatus}
                </div>
            )}
            <div>
                <Input
                    value={searchQuery}
                    onChange={(event) => onSearchQueryChange(event.target.value)}
                    placeholder="Search context key or value..."
                    data-testid="run-context-search-input"
                    className="text-sm"
                />
            </div>
            {contextError && (
                <InlineNotice tone="error" className="space-y-1">
                    <div data-testid="run-context-error">{contextError.message}</div>
                    <div data-testid="run-context-error-help" className="text-xs text-destructive/90">
                        {contextError.help}
                    </div>
                </InlineNotice>
            )}
            {!contextError && (
                <div className="overflow-hidden rounded-md border border-border/80">
                    <table data-testid="run-context-table" className="w-full table-fixed border-collapse text-sm">
                        <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                            <tr>
                                <th className="w-2/5 px-3 py-2 font-semibold">Key</th>
                                <th className="px-3 py-2 font-semibold">Value</th>
                            </tr>
                        </thead>
                        <tbody>
                            {filteredContextRows.length > 0 ? (
                                filteredContextRows.map((row) => (
                                    <tr key={row.key} data-testid="run-context-row" className="border-t border-border/70 align-top">
                                        <td className="px-3 py-2 font-mono text-xs text-foreground">{row.key}</td>
                                        <td className="space-x-2 px-3 py-2 font-mono text-xs text-foreground break-all">
                                            <span
                                                data-testid="run-context-row-type"
                                                className="inline-flex rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                                            >
                                                {row.valueType}
                                            </span>
                                            {row.renderKind === 'structured' ? (
                                                <div data-testid="run-context-row-value">
                                                    <pre
                                                        data-testid="run-context-row-value-structured"
                                                        className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-muted/40 px-2 py-1"
                                                    >
                                                        {row.renderedValue}
                                                    </pre>
                                                </div>
                                            ) : (
                                                <span data-testid="run-context-row-value">
                                                    <span data-testid="run-context-row-value-scalar">{row.renderedValue}</span>
                                                </span>
                                            )}
                                        </td>
                                    </tr>
                                ))
                            ) : (
                                <tr>
                                    <td data-testid="run-context-empty" colSpan={2} className="px-3 py-4 text-sm text-muted-foreground">
                                        No context entries match the current search.
                                    </td>
                                </tr>
                            )}
                        </tbody>
                    </table>
                </div>
            )}
                </PanelContent>
            ) : null}
        </Panel>
    )
}
