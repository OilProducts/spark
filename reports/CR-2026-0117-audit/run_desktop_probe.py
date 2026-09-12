#!/usr/bin/env python3
"""Compile/run the actual Desktop bootstrap assertion against audited libraries."""
from pathlib import Path
import os
import subprocess
import tempfile

here = Path(__file__).resolve().parent
worktree = here.parents[1] / ".spark/checkouts/run-18d45752f4ec6770"
deps = worktree / "target/debug/deps"
storage = max(deps.glob("libspark_storage-*.rlib"), key=lambda path: path.stat().st_mtime)
with tempfile.TemporaryDirectory(prefix="spark-desktop-precedence-audit-") as directory:
    binary = Path(directory) / "probe"
    subprocess.run([
        "rustc", "--edition=2021", str(here / "desktop_precedence.rs"),
        "--extern", f"spark_desktop={deps / 'libspark_desktop.rlib'}",
        "--extern", f"spark_storage={storage}", "-L", f"dependency={deps}",
        "-o", str(binary),
    ], check=True)
    environment = os.environ.copy()
    environment["SPARK_CLAUDE_CODE_CONFIG_DIR"] = "/tmp/audit-claude-config"
    result = subprocess.run([str(binary), str(Path(directory) / "state")], env=environment)
    raise SystemExit(result.returncode)
