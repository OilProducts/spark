from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping

from tests.compat import harness


REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")


def test_server_init_layout_fixture_records_catalog_and_seeded_flows(
    compat_fixture_root: Path,
) -> None:
    manifest = _load_storage_fixture(compat_fixture_root, "server-init-layout")
    spark_home_entries = _entry_paths(manifest["filesystem"]["after"]["spark_home"])
    flows_entries = _entry_paths(manifest["filesystem"]["after"]["flows"])

    assert "config/flow-catalog.toml" in spark_home_entries
    assert "examples/simple-linear.dot" in flows_entries
    assert "software-development/implement-change-request.dot" in flows_entries

    catalog = manifest["durable_state"]["after"]["flow_catalog"]
    assert catalog["format"] == "toml"
    assert "flows" in catalog["data"]


def test_flow_format_write_effect_fixture_records_file_rewrite(
    compat_fixture_root: Path,
) -> None:
    manifest = _load_storage_fixture(compat_fixture_root, "flow-format-write-effect")
    before = manifest["filesystem"]["before"]["project"]
    after = manifest["filesystem"]["after"]["project"]

    harness.assert_filesystem_effect(before, after, changed=("messy-write.dot",))
    formatted = manifest["durable_state"]["after"]["formatted_flow"]
    assert formatted["exists"] is True
    assert formatted["format"] == "bytes"


def test_project_conversation_and_run_request_fixtures_record_state_shapes(
    compat_fixture_root: Path,
) -> None:
    layout = _load_storage_fixture(compat_fixture_root, "project-conversation-layout")
    run_request = _load_storage_fixture(compat_fixture_root, "convo-run-request-state")

    layout_entries = _entry_paths(layout["filesystem"]["after"]["spark_home"])
    assert "workspace/conversation-handles.json" in layout_entries
    assert any(path.endswith("/project.toml") for path in layout_entries)
    assert any(path.endswith("/conversations/conversation-compat/state.json") for path in layout_entries)

    handles = layout["durable_state"]["after"]["conversation_handles"]["data"]
    assert handles["schema_version"] == 1
    assert handles["pattern"] == "adjective-noun"
    assert handles["handles"]
    assert "conversation-compat" in handles["conversation_ids"]

    request_state = run_request["durable_state"]["after"]["flow_run_requests"]["data"]
    assert request_state["conversation_id"] == "conversation-compat"
    assert request_state["flow_run_requests"]
    assert request_state["flow_run_requests"][0]["flow_name"].endswith(".dot")


def test_trigger_create_delete_fixtures_record_state_lifecycle(
    compat_fixture_root: Path,
) -> None:
    created = _load_storage_fixture(compat_fixture_root, "trigger-create-state")
    deleted = _load_storage_fixture(compat_fixture_root, "trigger-delete-state")

    created_entries = _entry_paths(created["filesystem"]["after"]["spark_home"])
    deleted_entries = _entry_paths(deleted["filesystem"]["after"]["spark_home"])
    created_definition_paths = [path for path in created_entries if path.startswith("config/triggers/")]

    assert created_definition_paths
    assert any(path.startswith("workspace/trigger-state/") for path in created_entries)
    assert not any(path.startswith("config/triggers/") for path in deleted_entries)
    assert not any(path.startswith("workspace/trigger-state/") for path in deleted_entries)

    trigger_definition = created["durable_state"]["after"]["trigger_definition"]
    trigger_state = created["durable_state"]["after"]["trigger_state"]
    assert trigger_definition["format"] == "toml"
    assert trigger_definition["data"]["source_type"] == "webhook"
    assert trigger_state["format"] == "json"
    assert "recent_history" in trigger_state["data"]


def _load_storage_fixture(root: Path, name: str) -> Mapping[str, Any]:
    manifest = harness.load_manifest(root / "filesystem" / f"{name}.json")
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
    return manifest


def _entry_paths(snapshot: Mapping[str, Any]) -> set[str]:
    return {
        str(entry["path"])
        for entry in snapshot.get("entries", [])
        if isinstance(entry, Mapping)
    }
