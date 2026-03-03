from __future__ import annotations

from pathlib import Path


def test_ui_no_op_save_paths_enforce_semantic_equivalence_item_5_3_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]

    flow_persistence_text = (repo_root / "frontend" / "src" / "lib" / "flowPersistence.ts").read_text(
        encoding="utf-8"
    )
    editor_text = (repo_root / "frontend" / "src" / "components" / "Editor.tsx").read_text(encoding="utf-8")

    assert "expect_semantic_equivalence" in flow_persistence_text
    assert "status === 'semantic_mismatch'" in flow_persistence_text
    assert "{ expectSemanticEquivalence: true }" in editor_text
    assert "const shouldExpectSemanticEquivalence = nonSelectChanges.length > 0" in editor_text
    assert "scheduleSave(nextNodes, edges, { expectSemanticEquivalence: true });" in editor_text
    assert "const expectSemanticEquivalence = rawDotEntryDraftRef.current === rawDotDraft;" in editor_text
    assert "const saveOptions = expectSemanticEquivalence ? { expectSemanticEquivalence: true } : undefined;" in editor_text
