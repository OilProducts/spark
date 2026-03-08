from __future__ import annotations

from argparse import Namespace
from pathlib import Path

import attractor.cli as cli


def test_run_serve_uses_import_string_when_reload_enabled(monkeypatch, tmp_path: Path) -> None:
    calls: list[dict[str, object]] = []

    def fake_run(app, **kwargs):
        calls.append({"app": app, **kwargs})

    monkeypatch.setattr("uvicorn.run", fake_run)

    args = Namespace(
        host="127.0.0.1",
        port=8000,
        reload=True,
        data_dir=tmp_path / "data",
        runs_dir=tmp_path / "runs",
        flows_dir=tmp_path / "flows",
        ui_dir=tmp_path / "ui",
        command="serve",
    )
    args.ui_dir.mkdir(parents=True, exist_ok=True)
    (args.ui_dir / "index.html").write_text("<html></html>", encoding="utf-8")

    result = cli._run_serve(args)

    assert result == 0
    assert calls == [
        {
            "app": "attractor.api.server:app",
            "host": "127.0.0.1",
            "port": 8000,
            "reload": True,
        }
    ]


def test_run_serve_preserves_runtime_path_env_for_reload(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr("uvicorn.run", lambda *args, **kwargs: None)

    data_dir = tmp_path / "data"
    runs_dir = tmp_path / "runs"
    flows_dir = tmp_path / "flows"
    ui_dir = tmp_path / "ui"
    ui_dir.mkdir(parents=True, exist_ok=True)
    (ui_dir / "index.html").write_text("<html></html>", encoding="utf-8")

    args = Namespace(
        host="127.0.0.1",
        port=8000,
        reload=True,
        data_dir=data_dir,
        runs_dir=runs_dir,
        flows_dir=flows_dir,
        ui_dir=ui_dir,
        command="serve",
    )

    cli._run_serve(args)

    assert cli.os.environ["SPARKSPAWN_HOME"] == str(data_dir.resolve(strict=False))
    assert cli.os.environ["SPARKSPAWN_RUNS_DIR"] == str(runs_dir.resolve(strict=False))
    assert cli.os.environ["SPARKSPAWN_FLOWS_DIR"] == str(flows_dir.resolve(strict=False))
    assert cli.os.environ["SPARKSPAWN_UI_DIR"] == str(ui_dir.resolve(strict=False))
