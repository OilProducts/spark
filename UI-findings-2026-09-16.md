**Spark Desktop — UI/UX audit**

September 16, 2026 · Native bundled app (`Spark.app`, `dev.spark.desktop`) driven directly on macOS · America/New_York

**Scope and method**

This pass was done on the real Tauri window rather than a browser tab, using the new `just run-app` bundle. I walked every navigation area (Home, Tasks, Editor, Triggers, Settings, Runs), the project picker, project settings, the notification popover, the task detail and creation panels, the flow editor in Structured, YAML, Parent and Expanded modes, the node inspector, the Launch Flow panel, and a completed run's Activity, Result and Details tabs. I looked at the default 1280 × 840 window and at the zoomed (near full-screen) window. I did not send chat messages, start or cancel runs, save settings, create tasks or triggers, or delete anything. Findings are tied to source files where I could locate the responsible code in `frontend/src`.

I read the September 9 evaluation (`UI-findings.md`) before writing this. Where a finding there is still open I reference it by its ID rather than restating it; the "Status of earlier findings" section at the end summarizes what has moved.

The brief was usability first, then style and general satisfyingness, for a power-user tool that should still be approachable to someone who isn't one yet. That framing drives the priorities: anything that makes a newcomer misread state or fear a click is P1; anything that costs an expert repeated effort is P2; polish is P3.

**Overall assessment**

The bones are good. Spark has a restrained, consistent visual language (shadcn-style cards, neutral palette, one accent for status), and the parts that matter most to a power user are already strong: the transcript's collapsible tool-call rows and "Worked for 33s" dividers, the run Result and Details tabs, the honest "Unpriced model usage," the auto-named threads, and a project picker that shows paths. Nothing here needs a redesign.

What holds it back is a layer of unfinished seams that a newcomer hits in the first minute and an expert trips over every day: a red error strip that never goes away and reads as one run-on sentence; graph edges drawn straight through node labels; a destructive red "Remove" button living in the global header; a chat screen where the message you are reading gets a third of the height the composer and its controls get; a Settings page that is one ten-screen scroll with seven independent Save buttons; and no dark mode at all on a platform where most developers run dark. Underneath those is a structural gap for the power-user promise: there is no keyboard layer. No command palette, no navigation shortcuts, no "focus composer," no "run flow." The app is entirely mouse-driven today.

The good news is that almost all of this is local, mechanical work. The P1s below are each a day or less.

**Prioritized findings**

| ID | Priority | Finding |
|---|---|---|
| G01 | P1 | Persistent preferences error strip is unstyled, unreadable, and global |
| G02 | P1 | Graph edges render through node labels; toolbar covers the start node; graph is not fitted |
| G03 | P1 | No dark mode; `.dark` tokens exist but are never applied |
| G04 | P2 | Destructive "Remove" project button sits in the global navbar |
| G05 | P2 | Home gives the transcript the least space of anything on screen |
| G06 | P2 | Composer controls are ambiguous, wrap, and the Send button reads as disabled |
| G07 | P2 | No keyboard layer: no palette, no shortcuts, no focus management |
| G08 | P2 | Settings is a single long scroll with seven Save buttons and raw keys as labels |
| G09 | P2 | Editor sidebar collapses the flow tree to four rows; mixed fonts; jargon modes; YAML view is raw |
| G10 | P2 | Form controls and dialog footers are inconsistent across features |
| G11 | P2 | Runs list rows are indistinguishable; "Cancel" offered on finished runs; counts unexplained |
| G12 | P3 | Project settings dialog uses the raw path as its title |
| G13 | P3 | Tasks board overflows horizontally at the default width |
| G14 | P3 | Triggers page uses a different content width and form-first layout |
| G15 | P3 | Badges that look like buttons ("CHAT MODE", "LIVE") |
| G16 | P3 | Typography and iconography details |

---

**G01 · P1 — Persistent preferences error strip is unstyled, unreadable, and global**

*Observed.* From launch, a red line sits above the app chrome on every screen: `Unable to load client preferences.Retry preference saveDiscard preference changes`. The two actions are bare `<button>` elements with no padding, border, or spacing, so the whole thing reads as one sentence. After visiting the Editor it changed to `Unable to save preferences. Your changes are retained.Retry preference saveDiscard preference changes` and stayed for the rest of the session. Settings → Client preferences also shows "Unable to refresh client preferences." and its selects render in the disabled grey style.

*Why it matters.* This is the first thing a new user sees, and it is the most visually loud element in the app. It says something is broken, offers two indistinguishable actions, and never resolves. A power user learns to ignore it, which means they will also ignore the next red strip that matters.

*Source.* `frontend/src/features/settings/ClientPreferencesController.tsx` lines 74–78: `<div role="alert" className="p-2 text-xs text-destructive">{error}<button …>Retry…</button><button …>Discard…</button></div>`. The buttons have no classes at all. The load failure itself comes from `loadAndMigrateClientPreferences()` in `services/clientPreferencesMigration.ts`; I reproduced it in the bundled build and did not verify whether the bare `cargo run` binary shows it too — worth checking, because if it is bundle-specific the fix is in `desktop_core.rs` paths, not the UI.

*Recommendation.* Two separate fixes. First, fix the underlying load (or, if the endpoint legitimately fails on a fresh app-data dir, treat "no stored preferences" as an empty set rather than an error). Second, replace the strip with the existing `components/ui/alert.tsx` in a destructive variant, buttons from `button.tsx` (`variant="outline" size="sm"`), a gap between them, and a dismiss control. Scope it: a load failure belongs inside Settings → Client preferences, and a save failure belongs next to the control that triggered the save (or as a toast), not above the navbar on every page.

*Acceptance.* A fresh install shows no error. When an error does occur, the two actions are visually separate buttons, the message says what will happen if you click each, and it can be dismissed.

---

**G02 · P1 — Graph edges render through node labels; toolbar covers the start node; graph is not fitted**

*Observed.* In the Editor with `examples/simple-linear.yaml`, a grey edge runs vertically through the centre of every node, visibly striking through "Pl|an", "Sum|marize" and "Do|ne". The floating toolbar (Structured / YAML / Parent / Expanded / + Node / Run) wraps to two rows at the default width and sits on top of the Start diamond, so the first node of every flow is hidden behind controls. The graph is left-of-centre with a large empty right half, and the Done node is clipped at the bottom. In Expanded mode the help line "Expanded child-flow mode is a read-only canvas preview…" also overlaps the diamond, and the minimap goes blank. On the Runs page the run graph shows the same edge-through-nodes behaviour plus a large blank white-and-grey rectangle covering the top of the graph. The September 9 finding F02 (clipped run graph, obstructive minimap) is still open.

*Why it matters.* The graph is the product's signature surface. Lines through labels make the flow look broken to a newcomer and make a real edge impossible to distinguish from a rendering bug for an expert. Hiding the start node behind the toolbar means every flow appears to begin mid-stream.

*Source.* Node frames are nearly opaque (`workflowNodeFrames.tsx`, `CARD_FILL = 'rgba(255,255,255,0.96)'`), so the visible line is an edge layered above the nodes or routed centre-to-centre. `features/workflow-canvas/ValidationEdge.tsx` falls back to `buildFallbackRoute(sourceX, sourceY, targetX, targetY)` — a straight centre-to-centre segment — whenever the ELK route is missing; in a vertical chain that segment passes through every intermediate node. Check which edge (likely the Done→Start loop) is taking the fallback, and confirm the edge layer is not elevated (`Editor.tsx` line 1476 sets `elevateEdgesOnSelect`). The toolbar is `EditorCanvasToolbar.tsx` line 41, an absolutely positioned wrapping flex box with `bg-background/90`. Fit is `fitView` on `Editor.tsx` line 1477 and `RunGraphCard.tsx` lines 193–194, evidently running before the panel has its final size.

*Recommendation.* Route the fallback around nodes (or at minimum draw it handle-to-handle with an offset, not centre-to-centre) and give edges a lower z-index than nodes. Move the toolbar out of the canvas into a fixed bar above it, or reserve top padding in `fitViewOptions` equal to the toolbar height so the fitted graph starts below it. Re-run `fitView` on container resize (a `ResizeObserver` on the canvas wrapper) so the graph is centred and fully visible at any panel size. Find the blank rectangle in the run graph (it looks like a node or overlay with no content rendering at the top-left) and either render it or remove it.

*Acceptance.* No edge crosses a node body. Every node, including Start, is visible after load without panning. The toolbar never covers graph content. The run graph shows no empty rectangles.

---

**G03 · P1 — No dark mode; `.dark` tokens exist but are never applied**

*Observed.* The app is white-on-white regardless of macOS appearance. `frontend/src/index.css` defines a complete `.dark` token set and a Tailwind `@custom-variant dark`, but nothing in `src/` ever adds the `dark` class or reads `prefers-color-scheme`.

*Why it matters.* This is the single biggest "satisfyingness" gap for the audience. A developer tool that ignores system appearance feels like a web page in a frame rather than a desktop app, and in a dark desktop the white window is physically uncomfortable in the evening, which is exactly when long agent runs get watched.

*Recommendation.* Cheapest version: in `App.tsx` (or `main.tsx`), add `document.documentElement.classList.toggle('dark', matchMedia('(prefers-color-scheme: dark)').matches)` and listen for changes. Then sweep the handful of hard-coded colours that will not follow tokens: `CARD_FILL` in `workflowNodeFrames.tsx`, the amber/slate/green pills in `TaskNode.tsx` (lines 427–432), the status badge colours, and the minimap `!important` overrides in `index.css`. Offer System / Light / Dark under Settings → Client preferences (which already has the plumbing for persisted preferences). Also set Tauri's window `theme` so the title bar matches.

*Acceptance.* Switching macOS appearance switches Spark, including the graph canvas, badges, and title bar, with no white flashes.

---

**G04 · P2 — Destructive "Remove" project button sits in the global navbar**

*Observed.* When a project is selected, the header shows `spark ▾ | + Add | × Clear | ☰ | 🗑 Remove`, with Remove in red. It is visible on every screen, one click away, adjacent to the two most-used buttons. "Clear" (which clears the active-project selection) reads as a sibling destructive action.

*Why it matters.* A newcomer will not click it because it looks dangerous, and will also not understand what Clear does. An expert will one day mis-click it while reaching for Add. Destructive actions belong behind a menu or in the object's own settings, not in persistent chrome.

*Source.* `frontend/src/app/Navbar.tsx` lines 340–385.

*Recommendation.* Move Remove into the project settings dialog (G12) as a "Remove project from Spark…" row at the bottom, with an alert dialog that names the path and states that files on disk are untouched (if that is true). Rename Clear to "Deselect" or fold it into the project picker as a "No project" entry, which is where the picker already shows "No active project selected." That leaves the header with the picker, Add, and the settings icon.

---

**G05 · P2 — Home gives the transcript the least space of anything on screen**

*Observed.* At the default 1280 × 840 window, the Project Chat card allots roughly 350 px to the transcript and roughly 240 px to the composer plus its two rows of controls and hint text. The left column splits into Threads (four visible rows, scroll for the rest) and Workflow Event Log (five rows). At the zoomed window it is better but the proportion is the same: the textarea is `h-[clamp(6rem,18vh,12rem)]` regardless of whether anything is typed (`ProjectConversationSurface.tsx` line 181).

*Why it matters.* The transcript is what you read; the composer is what you occasionally type into. On Home the reading area is the smallest region on screen, while an empty textarea takes a fixed 18 % of the viewport. Threads is the navigation for the whole conversation model and shows four items.

*Recommendation.* Let the textarea start at two lines and grow with content to a max (`field-sizing: content` is already half-used here; drop the `[field-sizing:fixed]` override). Fold the controls row into one line (see G06). Give Threads the full left column by default and move the Workflow Event Log behind the bell icon or into a collapsible strip at the bottom; it is a feed, not a navigation surface, and its "All projects / This project" toggle duplicates the Runs scope control. Keep the drag divider for people who want both.

---

**G06 · P2 — Composer controls are ambiguous, wrap, and the Send button reads as disabled**

*Observed.* Below the textarea: `Press Enter to send. Use Shift+Enter for a new line.` on the left; on the right `Conversation override  Use defaults  [Codex ▾] [GPT-6-Astra ▾]` and, wrapped to a second row, `[Medium ▾]  Send`. "Use defaults" is set in a heavier weight than "Conversation override" and it is not obvious which is a label and which is a button. "Send" is rendered in the ghost/outline style with grey text, so it looks disabled even when the textarea is empty by design and would look the same when it isn't. The model select showed "Loading models…" for several seconds on first visit.

*Why it matters.* The three dropdowns encode the same thing as Settings → Model defaults but with different labels (chat says "Codex", Settings says `codex`); the earlier finding F13 covers the inheritance confusion. Visually, the two-row wrap makes the composer look unfinished, and the hint text competes with the controls.

*Recommendation.* Collapse provider, model and reasoning into one chip — `Codex · GPT-6-Astra · Medium ▾` — that opens a popover with the three selects and a "Reset to defaults" link; that also gives "Conversation override" a home as the popover title. Make Send a filled primary button (or an icon button inside the textarea's bottom-right corner) that is disabled only when there is nothing to send. Move the Enter/Shift+Enter hint into the textarea placeholder or a tooltip on Send.

---

**G07 · P2 — No keyboard layer: no palette, no shortcuts, no focus management**

*Observed.* ⌘K does nothing. There are no shortcuts for switching sections, creating a thread, focusing the composer, opening a flow, or running one. Landing on Home does not focus the composer. `grep` for `metaKey|ctrlKey|keydown` across `frontend/src` finds only `features/runs/RunsPanel.tsx`.

*Why it matters.* This is the gap between "a tool power users can use" and "a power-user tool." Everything else in this list is polish by comparison. It is also the accessibility baseline: the September 9 finding F03 (inspector fields without labels) is still open and would be caught by the same keyboard pass.

*Recommendation.* Add a command palette (cmdk works well with the existing Radix stack) seeded with navigation, projects, threads, saved flows, and recent runs; that alone answers the earlier search findings (F04) without designing search boxes into every list. Then a small fixed set: ⌘1–6 for sections, ⌘N new thread, ⌘⇧R run current flow, ⌘/ focus composer, ⌘, Settings, Esc closes panels. Show them in tooltips and in the palette. Autofocus the composer when Home loads with a project selected, and the title field when a task or trigger form opens (the task form already does this — copy that).

---

**G08 · P2 — Settings is a single long scroll with seven Save buttons and raw keys as labels**

*Observed.* Settings runs roughly ten screens at the default height with no section navigation: Codex connection, Model defaults (Workspace), Scoped configuration, Project model defaults, Runtime paths, Server and client connections, Client preferences, Provider connections (one block per provider), Agent settings, Execution profiles. Each section has its own `Save … / Discard … changes` pair; the Save buttons render in the disabled grey state until something changes, so the page is dotted with grey pills. Provider connections exposes raw keys as labels: `openai`, `base url`, `organization`, `project` in lowercase beside `Credential environment variable` in sentence case. Client preferences exposes internal layout state as numeric fields: "Editor sidebar width (pixels)", "Home sidebar primary split (0–1)", "Run graph height". Execution profiles ends with an unguarded "Delete execution profile native" button.

*Why it matters.* An expert who wants to change one thing scrolls and reads to find it; a newcomer cannot tell what is safe to touch. Layout numbers in a form are a tell that state is being surfaced because it exists, not because a person needs to set it.

*Source.* `features/settings/SettingsPanel.tsx` composes the editors; labels are in `ProviderSettingsEditor.tsx`; the layout fields are in `ClientPreferencesEditor.tsx`.

*Recommendation.* Add a left rail of section links (sticky, scroll-spy) or convert to tabs; the content width is already narrow enough to leave room. Keep per-section saves if the revision-checked backend needs them, but hide Save/Discard until the section is dirty and show a single "Unsaved changes in Model defaults" pill in the rail. Title-case labels from a display-name map rather than the config key. Drop the sidebar-width and split-ratio fields from the form and persist them from the drag interactions that already exist; if a reset is wanted, a single "Reset layout" button covers it. Put the profile delete behind an alert dialog and disable it when the profile is in use.

---

**G09 · P2 — Editor sidebar collapses the flow tree to four rows; mixed fonts; jargon modes; YAML view is raw**

*Observed.* Opening a flow, and again selecting a node, pushes an inspector card into the bottom of the left column, leaving the Saved Flows tree four rows tall with its own scrollbar. The tree sets folder names in monospace and file names in the proportional face, with top-level files indented less than nested files. The panel header cycles `FLOWS ●` → `GRAPH ●` → `NODE ●` with no explanation of the dot. The toolbar's `Parent | Expanded` toggle needs the help text "Switch to Parent Only to edit" to be understood. YAML mode opens scrolled to the middle of the document, with no line numbers or highlighting, and serialises every default (`condition: ""`, `fidelity: ""`, `thread_id: ""`, `weight: 0` on every edge). In YAML mode the node inspector's Label field went blank while the same node showed "Implement" in Structured mode.

*Why it matters.* The editor is where flows are authored and the left column is being asked to be both a file browser and a property inspector at the same time. The earlier finding F16 (implementation language leaking into the editor) still stands.

*Recommendation.* Move the inspector to a right-hand panel, the way Tasks already does for task detail, and let the tree keep the left column. One font for the tree (proportional, with a folder icon doing the work), consistent indentation per depth, and a plain header — "Flows" — with the mode shown as a chip on the toolbar ("Editing node: Implement"). Rename Parent/Expanded to "Edit" and "Preview with child flows". For YAML, reuse highlight.js (already a dependency for markdown), add line numbers, open at the top, and emit only non-default keys, keeping full round-tripping on load. Investigate the blank Label in YAML mode as a state-sync bug.

---

**G10 · P2 — Form controls and dialog footers are inconsistent across features**

*Observed.* Three checkbox styles: the custom black `Enabled` in Triggers, the native blue `Enabled` in Settings → Execution profiles, and a Switch for "Override workspace model settings." Two select styles (`native-select.tsx` and `select.tsx`) side by side in Settings. The task detail uses a native `<details>` disclosure ("▶ Activity and details") in the browser's default style. Footers offer two dismiss actions: New task has `Create task | Cancel | Discard changes`; project settings has `Discard and reload | Cancel | Save`; every Settings section has `Save | Discard`. Section filters use three different chip styles for the same job (Home's black/white toggle, Runs' outlined pair, Triggers' outlined pair with different spacing).

*Why it matters.* Each inconsistency is small; together they are the reason the app feels assembled rather than designed, and they teach a newcomer that the same-looking thing might behave differently.

*Recommendation.* Pick one of each from `components/ui` and delete the others: `checkbox.tsx` everywhere a boolean is a form field, `switch.tsx` only for immediate-effect toggles, `select.tsx` everywhere, `collapsible.tsx` for disclosures, `tabs.tsx` (or a single segmented-control component) for scope filters. Footers: one primary plus Cancel; show a Discard only while a draft is dirty, and never both Cancel and Discard.

---

**G11 · P2 — Runs list rows are indistinguishable; "Cancel" offered on finished runs; counts unexplained**

*Observed.* The Runs sidebar truncates every row to `software-deve…` or `Implement…`; eight of nine visible rows read identically apart from their duration. The header shows `148 runs · 0 running` and `RECENT 47` without saying what the two numbers count. The selected completed run's header offers `Re-run | Continue from node | Cancel`, with Cancel enabled-looking on a run that finished four days ago. The earlier finding F04 covers the identification problem in depth and is still open.

*Recommendation.* Widen the sidebar (it has room; the detail pane is wide), show the run's objective or flow display name on line one and the ID/duration on line two, and hide Cancel for terminal runs. Label the counts ("47 root runs · 148 including child runs" or whatever the relationship is).

---

**G12 · P3 — Project settings dialog uses the raw path as its title**

*Observed.* The dialog opened from the ☰ icon is titled `/Users/chris/projects/spark` with "Project settings" as the subtitle, contains one field (Default execution profile), and offers `Discard and reload | Cancel | Save`.

*Recommendation.* Title it with the project name, path as subtitle; make it the home for Remove project (G04) and the project's git metadata the app already fetches (`useProjectGitMetadata.ts`); footer becomes Cancel + Save.

---

**G13 · P3 — Tasks board overflows horizontally at the default width**

*Observed.* Six columns at fixed width; Review is clipped and anything after it is off-screen with no scroll affordance. Opening a task pushes the board further. The `Tasks | Create task | Search titles [ ] | Show archived | ⟳` toolbar reads as one run-on line because the page title sits in the same row as the primary action.

*Recommendation.* Let columns shrink to a min width and horizontally scroll with a visible shadow at the edge, or collapse empty columns to a narrow strip with the count. Put the page title on its own line or drop it (the nav already says Tasks). The Tasks work since September 9 (bordered buttons, side-panel creation, Cancel) is a clear improvement; F05 can be closed.

---

**G14 · P3 — Triggers page uses a different content width and form-first layout**

*Observed.* Triggers is inset with a wider margin than Tasks or Home and centres a two-column layout that no other page uses. The Create trigger form is always expanded, and an empty "Selected trigger" card sits below it. Target Flow is still a free-text field and Interval is still raw seconds (F11 open).

*Recommendation.* Match the page frame used by Tasks and Runs. Show the list first with a "New trigger" button that opens the form in the same right-side panel pattern as Tasks; hide "Selected trigger" until one is selected.

---

**G15 · P3 — Badges that look like buttons**

*Observed.* `CHAT MODE` in the chat header is an outlined pill in button styling that does nothing on click. `LIVE` on the run Activity tab is a bordered blue pill on a run that finished days ago (F08 open).

*Recommendation.* Use `badge.tsx` without a border for status, reserve the outlined pill for things that respond to clicks, and rename LIVE to "Following" only while a stream is actually open.

---

**G16 · P3 — Typography and iconography details**

*Observed.* `index.css` names Inter as the primary face but nothing loads it, so the app renders in the system font (SF on macOS, which is fine; on Linux and Windows the fallback will vary). Monospace is used for thread slugs, folder names, and the Node ID chip but not consistently for paths (the project settings title is proportional; the Launch panel's flow path is monospace). The notification popover opens left-aligned to the bell and extends into empty header space. The new bundle now has a proper Dock icon; the old 128 px placeholder should not be reintroduced.

*Recommendation.* Either bundle Inter with the frontend or remove it from the stack so behaviour is deliberate. One rule for monospace: identifiers and paths, nothing else.

---

**What is working well**

The transcript is the best part of the app. Tool calls collapse to one line with a status pill and the command, user turns sit right in a grey card, assistant turns sit left, and "Worked for 33s" dividers make long agent sessions scannable. Keep this exactly as it is and reuse its conventions in the run Activity tab (which already does).

Run Result and Details are well organised. Details' label-value grid (Outcome, Scope, Execution, Usage) is the model the rest of Settings should follow. "Unpriced model usage" is an honest answer where most tools would print $0.00.

The project picker shows name and path together, threads get memorable auto-names (`summer-barley`, `stone-ash`), the empty attention popover says "Nothing is waiting on you" in plain language, and drafts survive navigation. The Launch Flow panel explains its execution profile in context. The palette is calm, the card grammar is consistent, and the status colours (green completed, amber cancelled, red failed) are used sparingly enough to mean something.

**Status of earlier findings (September 9)**

Improved and can be closed: F05 (Tasks controls now match the rest of the app; creation is a side panel with Cancel) and F07 (markdown tables and lists now render in the transcript).

Still open, confirmed in this pass: F02 (run graph fit and minimap), F03 (inspector field labels; not re-tested but the inspector code has not changed in a way that would fix it), F04 (run identification), F08 (LIVE badge and Activity as default), F10 (scope wording), F11 (trigger form), F13 (model inheritance; see G06), F14 (empty states), F16 (editor naming; see G09).

Not re-tested: F01 (narrow windows), F06 (task relationship IDs), F09 (recovery choices), F12 (artifact ordering), F15 (project onboarding).

**Suggested order of work**

Fix G01 (error strip) and G02 (edges and toolbar) first; both are a day each and they are what a newcomer sees in the first minute. G03 (dark mode) is a day for the toggle and an afternoon for the colour sweep, and it is the single change most likely to make the app feel finished. Then G07 (command palette and shortcuts), which pays back every day and quietly resolves the search findings. G04, G05 and G06 are one focused pass over Home and the navbar. G08 through G11 are a second pass over Settings, Editor and Runs. The P3s are a polish afternoon at the end.
