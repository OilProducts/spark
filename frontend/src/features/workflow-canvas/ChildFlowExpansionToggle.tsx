import { SegmentedControl } from '@/components/app/segmented-control'

export function ChildFlowExpansionToggle({
    expanded,
    onChange,
    disabled = false,
    testId,
}: {
    expanded: boolean
    onChange: (expanded: boolean) => void
    disabled?: boolean
    testId?: string
}) {
    return (
        <SegmentedControl
            data-testid={testId}
            aria-label="Child flow view"
            className="shrink-0"
            options={[
                { key: 'parent', label: 'Parent only', pressed: !expanded, onPress: () => onChange(false), disabled, className: 'flex-none' },
                { key: 'expanded', label: 'Child flows', pressed: expanded, onPress: () => onChange(true), disabled, className: 'flex-none' },
            ]}
        />
    )
}
