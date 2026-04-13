from __future__ import annotations

from pathlib import Path
import subprocess


def _tracked_files(repo_root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repo_root,
        check=True,
        capture_output=True,
    )
    return [repo_root / entry.decode("utf-8") for entry in result.stdout.split(b"\0") if entry]


def _read_tracked_text_files(repo_root: Path) -> list[tuple[Path, str]]:
    text_files: list[tuple[Path, str]] = []
    for path in _tracked_files(repo_root):
        if not path.exists():
            continue
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        text_files.append((path, content))
    return text_files


def test_tracked_files_do_not_leak_personal_local_paths() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    forbidden_values = (
        "/Users/" "chris/projects/spark",
        "/Users/" "chris/projects/agent-skills",
    )

    offenders: list[str] = []
    for path, content in _read_tracked_text_files(repo_root):
        for forbidden in forbidden_values:
            if forbidden in content:
                offenders.append(f"{path.relative_to(repo_root)}: {forbidden}")

    assert not offenders, "tracked files contain personal local paths:\n" + "\n".join(sorted(offenders))


def test_public_compose_file_does_not_mount_personal_codex_state() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    compose_text = (repo_root / "compose.yaml").read_text(encoding="utf-8")

    forbidden_mounts = (
        "${HOME}/.codex/auth.json",
        "${HOME}/.codex/config.toml",
    )
    offenders = [mount for mount in forbidden_mounts if mount in compose_text]

    assert not offenders, "compose.yaml mounts personal Codex state:\n" + "\n".join(offenders)
