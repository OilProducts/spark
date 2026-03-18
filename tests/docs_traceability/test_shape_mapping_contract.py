from pathlib import Path

from attractor.dsl import parse_dot
from attractor.handlers.defaults import build_default_registry
from attractor.handlers.registry import SHAPE_TO_TYPE


def _shape_mapping_from_spec() -> dict[str, str]:
    spec_path = Path(__file__).resolve().parents[2] / "specs/attractor-spec.md"
    lines = spec_path.read_text(encoding="utf-8").splitlines()

    start_idx = lines.index("### 2.8 Shape-to-Handler-Type Mapping")
    mapping: dict[str, str] = {}

    for line in lines[start_idx + 1 :]:
        stripped = line.strip()
        if stripped.startswith("### "):
            break
        if not (stripped.startswith("|") and stripped.endswith("|")):
            continue

        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if len(cells) < 2:
            continue
        if cells[0] == "Shape" or set(cells[0]) == {"-"}:
            continue

        shape = cells[0].strip("`")
        handler_type = cells[1].strip("`")
        mapping[shape] = handler_type

    return mapping


def test_shape_mapping_docs_table_matches_runtime_mapping():
    assert _shape_mapping_from_spec() == SHAPE_TO_TYPE


def _mixed_case(raw: str) -> str:
    return "".join(char.upper() if idx % 2 == 0 else char.lower() for idx, char in enumerate(raw))


def test_parser_normalizes_all_spec_shapes_for_registry_resolution():
    spec_mapping = _shape_mapping_from_spec()
    registry = build_default_registry()

    for canonical_shape, expected_handler_type in spec_mapping.items():
        graph = parse_dot(
            f"""
            digraph G {{
                stage [shape="  {_mixed_case(canonical_shape)}  "]
            }}
            """
        )

        assert graph.nodes["stage"].attrs["shape"].value == canonical_shape
        assert registry.resolve_handler_type(graph.nodes["stage"]) == expected_handler_type
