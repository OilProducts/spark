from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


EXIT_RUST_BINARY_UNAVAILABLE = 127


def spark_main() -> int:
    return _dispatch("spark", rust_package="spark-cli")


def spark_server_main() -> int:
    return _dispatch("spark-server", rust_package="spark-server")


def _dispatch(command_name: str, *, rust_package: str) -> int:
    binary_path = _rust_binary_path(command_name)
    if binary_path is None:
        return _fail_missing_rust_binary(command_name, rust_package=rust_package)
    argv = [str(binary_path), *sys.argv[1:]]
    if os.name == "posix":
        os.execv(str(binary_path), argv)
    completed = subprocess.run(argv, check=False)
    return int(completed.returncode)


def _rust_binary_path(command_name: str) -> Path | None:
    return _packaged_binary_path(command_name) or _source_checkout_binary_path(command_name)


def _packaged_binary_path(command_name: str) -> Path | None:
    suffix = ".exe" if os.name == "nt" else ""
    candidate = Path(__file__).resolve().parent / "bin" / f"{command_name}{suffix}"
    if _is_runnable_file(candidate):
        return candidate
    return None


def _source_checkout_binary_path(command_name: str) -> Path | None:
    workspace_root = _source_checkout_root()
    if workspace_root is None:
        return None

    suffix = ".exe" if os.name == "nt" else ""
    binary_name = f"{command_name}{suffix}"
    seen_target_dirs: set[Path] = set()
    for target_dir in _workspace_target_dirs(workspace_root):
        normalized_target_dir = target_dir.resolve(strict=False)
        if normalized_target_dir in seen_target_dirs:
            continue
        seen_target_dirs.add(normalized_target_dir)
        for profile in ("debug", "release"):
            candidate = normalized_target_dir / profile / binary_name
            if _is_runnable_file(candidate):
                return candidate
    return None


def _workspace_target_dirs(workspace_root: Path) -> tuple[Path, ...]:
    configured_target = os.environ.get("CARGO_TARGET_DIR", "").strip()
    target_dirs: list[Path] = []
    if configured_target:
        configured_path = Path(configured_target).expanduser()
        if not configured_path.is_absolute():
            configured_path = (Path.cwd() / configured_path).resolve(strict=False)
        target_dirs.append(configured_path)
    target_dirs.append(workspace_root / "target")
    return tuple(target_dirs)


def _source_checkout_root() -> Path | None:
    for directory in Path(__file__).resolve().parents:
        if _is_spark_rust_workspace(directory):
            return directory
    return None


def _is_spark_rust_workspace(directory: Path) -> bool:
    return (
        (directory / "Cargo.toml").is_file()
        and (directory / "pyproject.toml").is_file()
        and (directory / "crates" / "spark-cli" / "Cargo.toml").is_file()
        and (directory / "crates" / "spark-server" / "Cargo.toml").is_file()
    )


def _is_runnable_file(path: Path) -> bool:
    if not path.is_file():
        return False
    if os.name == "posix" and not os.access(path, os.X_OK):
        return False
    return True


def _fail_missing_rust_binary(command_name: str, *, rust_package: str) -> int:
    workspace_root = _source_checkout_root()
    command = _display_command(command_name)
    lines = [
        f"Spark could not find the Rust `{command_name}` binary required by `{command}`.",
        "",
        "The Python entry point is only a Rust launcher; it does not fall back to the Python Spark CLI, server, or provider runtime.",
    ]
    if workspace_root is not None:
        lines.extend(
            [
                "",
                f"Build the Rust command surface from this source checkout, then retry `{command}`:",
                "",
                f"  cd {workspace_root}",
                f"  cargo build -p {rust_package} --bin {command_name}",
                f"  {command}",
                "",
                "Use `cargo build --workspace` when you want to refresh all workspace binaries.",
            ]
        )
    else:
        lines.extend(
            [
                "",
                "This install is missing the packaged Rust payload. Reinstall Spark from a wheel that includes `spark/bin/*`.",
            ]
        )
    sys.stderr.write("\n".join(lines) + "\n")
    return EXIT_RUST_BINARY_UNAVAILABLE


def _display_command(command_name: str) -> str:
    args = " ".join(sys.argv[1:])
    return f"uv run {command_name} {args}".strip()
