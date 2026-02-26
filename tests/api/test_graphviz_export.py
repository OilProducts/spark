from pathlib import Path

import pytest

from attractor.graphviz_export import export_graphviz_artifact


def test_export_graphviz_artifact_writes_dot_and_svg_when_dot_succeeds(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    dot_source = "digraph G { start -> end; }"

    def fake_run(cmd, *, check, capture_output, text):
        output_flag_index = cmd.index("-o")
        svg_path = Path(cmd[output_flag_index + 1])
        svg_path.write_text("<svg>ok</svg>", encoding="utf-8")

        class Result:
            returncode = 0
            stderr = ""

        return Result()

    monkeypatch.setattr("subprocess.run", fake_run)

    result = export_graphviz_artifact(dot_source, tmp_path)

    assert result.dot_path.exists()
    assert result.dot_path.read_text(encoding="utf-8") == dot_source
    assert result.rendered_path is not None
    assert result.rendered_path.exists()
    assert result.rendered_path.name.endswith(".svg")
    assert result.error == ""


def test_export_graphviz_artifact_keeps_dot_when_graphviz_missing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    dot_source = "digraph G { start -> end; }"

    def missing_dot(*args, **kwargs):
        raise FileNotFoundError("dot not found")

    monkeypatch.setattr("subprocess.run", missing_dot)

    result = export_graphviz_artifact(dot_source, tmp_path)

    assert result.dot_path.exists()
    assert result.rendered_path is None
    assert result.error
    assert "dot" in result.error.lower()
