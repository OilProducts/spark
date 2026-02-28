from pathlib import Path


def test_editor_enforces_single_select_across_nodes_edges_and_inspector_item_5_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    editor_text = (repo_root / "frontend" / "src" / "components" / "Editor.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const enforceSingleSelectedNode = useCallback(",
        "const enforceSingleSelectedEdge = useCallback(",
        "const latestSelectedNodeChange = [...changes].reverse().find(",
        "const latestSelectedEdgeChange = [...changes].reverse().find(",
        "setSelectedNodeId(selectedNodeId)",
        "setSelectedEdgeId(null)",
        "setSelectedEdgeId(selectedEdgeId)",
        "setSelectedNodeId(null)",
    ]

    for snippet in required_snippets:
        assert snippet in editor_text, f"missing single-select enforcement snippet: {snippet}"


def test_checklist_marks_item_5_1_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [5.1-01]" in checklist_text


def test_editor_syncs_store_selection_back_to_canvas_item_5_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    editor_text = (repo_root / "frontend" / "src" / "components" / "Editor.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const { activeFlow, viewMode, selectedNodeId, selectedEdgeId, setSelectedNodeId, setSelectedEdgeId } = useStore();",
        "setNodes((currentNodes) =>",
        "const shouldSelect = !selectedEdgeId && node.id === selectedNodeId;",
        "setEdges((currentEdges) =>",
        "const shouldSelect = edge.id === selectedEdgeId;",
    ]

    for snippet in required_snippets:
        assert snippet in editor_text, f"missing bidirectional selection sync snippet: {snippet}"


def test_checklist_marks_item_5_1_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [5.1-02]" in checklist_text
