# Design System

## Direction

Personality: Utility & Function
Foundation: neutral
Depth: borders-only

## Tokens

### Spacing
Base: 4px
Scale: [4, 8, 12, 16, 24, 32, 48, 64]

### Colors
Foreground: #0f172a
Secondary: #1e293b
Muted: #475569
Accent: #2563eb
Surface-1: #ffffff
Surface-2: #f1f5f9
Border: #e2e8f0

### Radius
Scale: [4px, 8px]

### Typography
Primary font: Inter, system-ui, -apple-system, BlinkMacSystemFont
Mono font: "JetBrains Mono", "SFMono-Regular", ui-monospace
Scale: [12, 14, 16, 18, 24, 32]
Weights: [400, 500, 600, 700]

## Patterns

### Button Primary
- Height: 40px
- Padding: 0 16px
- Radius: 8px
- Typography: 14px / 600
- Background: Accent (#2563eb)
- Border/shadow: none, rely on color and hover state
- States: hover (#1d4ed8), focus (outline accent), active (#1e3a8a), disabled (opacity 0.6)

### Input Default
- Height: 40px
- Padding: 0 12px
- Radius: 8px
- Border: 1px solid Border token
- States: focus (border accent + shadow ring), error (border #dc2626), disabled (bg Surface-2)

### Card Default
- Padding: 24px
- Radius: 12px
- Border/shadow: 1px solid Border token
- Surface: Surface-1

## Decisions

| Decision | Rationale | Date |
|----------|-----------|------|
| Borders-only depth | Keeps focus on dense data tables without visual noise | 2026-02-22 |
| 4px spacing base | Offers grid-friendly sizing for tables/forms | 2026-02-22 |
