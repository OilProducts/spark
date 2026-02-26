from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import subprocess
from typing import Optional


@dataclass(frozen=True)
class GraphvizArtifactExport:
    dot_path: Path
    rendered_path: Optional[Path]
    error: str = ""


def export_graphviz_artifact(dot_source: str, run_root: Path) -> GraphvizArtifactExport:
    graph_dir = run_root / "artifacts" / "graphviz"
    graph_dir.mkdir(parents=True, exist_ok=True)

    dot_path = graph_dir / "pipeline.dot"
    dot_path.write_text(dot_source, encoding="utf-8")

    rendered_path = graph_dir / "pipeline.svg"
    try:
        subprocess.run(
            ["dot", "-Tsvg", str(dot_path), "-o", str(rendered_path)],
            check=True,
            capture_output=True,
            text=True,
        )
        return GraphvizArtifactExport(dot_path=dot_path, rendered_path=rendered_path)
    except FileNotFoundError:
        return GraphvizArtifactExport(
            dot_path=dot_path,
            rendered_path=None,
            error="Graphviz 'dot' binary not found",
        )
    except subprocess.CalledProcessError as exc:
        stderr = (exc.stderr or "").strip()
        message = stderr or str(exc)
        return GraphvizArtifactExport(
            dot_path=dot_path,
            rendered_path=None,
            error=f"Graphviz render failed: {message}",
        )
