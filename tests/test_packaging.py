from __future__ import annotations

import subprocess
import sys
import zipfile
from pathlib import Path


def test_built_wheel_includes_unified_llm_model_catalog(tmp_path: Path) -> None:
    wheelhouse = tmp_path / "wheelhouse"
    wheelhouse.mkdir()

    subprocess.run(
        [
            sys.executable,
            "-m",
            "pip",
            "wheel",
            "--no-deps",
            "--no-build-isolation",
            "--wheel-dir",
            str(wheelhouse),
            ".",
        ],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    wheel_paths = list(wheelhouse.glob("spark-*.whl"))
    assert len(wheel_paths) == 1

    with zipfile.ZipFile(wheel_paths[0]) as wheel:
        assert "unified_llm/data/models.json" in wheel.namelist()
