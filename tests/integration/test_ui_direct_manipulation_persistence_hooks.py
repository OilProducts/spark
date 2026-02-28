from pathlib import Path


def test_editor_supports_direct_manipulation_with_persistence_hooks_item_5_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    editor_text = (repo_root / "frontend" / "src" / "components" / "Editor.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {",
        "const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {",
        "const onConnect = useCallback(",
        "applyNodeChanges(changes, currentNodes)",
        "applyEdgeChanges(changes, currentEdges)",
        "const onSelectionChange = useCallback(({ nodes, edges }: OnSelectionChangeParams) => {",
        "const selectedNode = nodes.find(n => n.selected);",
        "const selectedEdge = edges.find(e => e.selected);",
        "scheduleSave(nextNodes, edges);",
        "scheduleSave(nodes, nextEdges);",
        "scheduleSave(nodes, newEdges);",
        "void saveFlowContent(activeFlow, dot);",
        "onNodesChange={onNodesChange}",
        "onEdgesChange={onEdgesChange}",
        "onConnect={onConnect}",
        "onSelectionChange={onSelectionChange}",
    ]

    for snippet in required_snippets:
        assert snippet in editor_text, f"missing direct manipulation + persistence snippet: {snippet}"


def test_checklist_marks_item_5_2_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [5.2-01]" in checklist_text
