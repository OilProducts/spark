from __future__ import annotations

from pathlib import Path
import tomllib


REPO_ROOT = Path(__file__).resolve().parents[2]


def test_rust_workspace_exposes_only_documented_public_binaries() -> None:
    workspace = _load_toml(REPO_ROOT / "Cargo.toml")
    members = set(workspace["workspace"]["members"])

    assert "crates/spark-cli" in members
    assert "crates/spark-server" in members

    binaries: dict[str, str] = {}
    for member in members:
        cargo_toml = REPO_ROOT / member / "Cargo.toml"
        if not cargo_toml.exists():
            continue
        crate = _load_toml(cargo_toml)
        package_name = crate["package"]["name"]
        for binary in crate.get("bin", []):
            binaries[binary["name"]] = package_name

    assert binaries == {
        "spark": "spark-cli",
        "spark-server": "spark-server",
    }


def test_python_package_entry_points_preserve_documented_command_names() -> None:
    pyproject = _load_toml(REPO_ROOT / "pyproject.toml")
    scripts = pyproject["project"]["scripts"]

    assert scripts == {
        "spark": "spark._rust_launcher:spark_main",
        "spark-server": "spark._rust_launcher:spark_server_main",
    }


def _load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))
