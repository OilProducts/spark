from __future__ import annotations

import base64
import csv
from dataclasses import dataclass
import hashlib
import io
from pathlib import Path
import os
import stat
import shutil
import subprocess
import sysconfig
import tempfile
import tarfile
import tomllib
import zipfile


FRONTEND_BINARIES = ("tsc", "vite")
RUST_BINARY_TARGETS = (
    ("spark-cli", "spark"),
    ("spark-server", "spark-server"),
)
REQUIRED_RUST_BINARY_ENTRIES = (
    "spark/bin/spark",
    "spark/bin/spark-server",
)
FORBIDDEN_SDIST_BINARY_ENTRIES = (
    *REQUIRED_RUST_BINARY_ENTRIES,
    "src/spark/bin/spark",
    "src/spark/bin/spark-server",
)
REQUIRED_SOURCE_PATHS = (
    "Cargo.toml",
    "Cargo.lock",
    "Dockerfile",
    "Dockerfile.wheel",
    "compose.package.yaml",
    "pyproject.toml",
    "src",
    "crates/spark-cli/src/main.rs",
    "crates/spark-server/src/main.rs",
    "crates/spark-assets/build.rs",
    "tests/compat/providers/test_unified_llm_adapter_fixtures.py",
    "tests/compat/packaging/test_packaging_smoke_fixtures.py",
    "frontend/package.json",
    "frontend/package-lock.json",
    "scripts/build_deliverable.py",
    "scripts/package-entrypoint.sh",
    "scripts/run-docker.sh",
)
EXCLUDED_DIR_NAMES = {
    ".git",
    ".pytest_cache",
    ".ruff_cache",
    ".spark",
    ".venv",
    "__tests__",
    "__pycache__",
    "dist",
    "node_modules",
    "target",
    "tests",
}
EXCLUDED_RELATIVE_DIRS = {
    Path("frontend") / "artifacts",
    Path("frontend") / "dist",
    Path("frontend") / "e2e",
    Path("frontend") / "node_modules",
    Path("frontend") / "playwright-report",
    Path("frontend") / "src" / "test",
    Path("frontend") / "test-results",
}
EXCLUDED_FILE_SUFFIXES = (".pyc", ".pyo")
REQUIRED_WHEEL_ENTRIES = (
    *REQUIRED_RUST_BINARY_ENTRIES,
    "spark/ui_dist/index.html",
    "spark/guides/dot-authoring.md",
    "spark/guides/spark-operations.md",
    "spark/flows/examples/simple-linear.dot",
    "spark/flows/software-development/spec-implementation/implement-spec.dot",
    "unified_llm/data/models.json",
)
REQUIRED_SDIST_SOURCE_ENTRIES = (
    "Cargo.toml",
    "Cargo.lock",
    "Dockerfile",
    "pyproject.toml",
    "Dockerfile.wheel",
    "compose.package.yaml",
    "scripts/build_deliverable.py",
    "scripts/package-entrypoint.sh",
    "scripts/run-docker.sh",
    "crates/spark-cli/src/main.rs",
    "crates/spark-server/src/main.rs",
    "crates/spark-assets/build.rs",
    "crates/spark-assets/src/lib.rs",
    "crates/spark-workspace/src/models.rs",
    "crates/unified-llm-adapter/src/env.rs",
    "crates/unified-llm-adapter/src/catalog.rs",
    "crates/unified-llm-adapter/src/profiles.rs",
    "frontend/package.json",
    "frontend/package-lock.json",
    "frontend/src/main.tsx",
    "frontend/vite.config.ts",
    "src/spark/flows/examples/simple-linear.dot",
    "src/spark/guides/dot-authoring.md",
    "src/unified_llm/data/models.json",
    "tests/compat/providers/test_unified_llm_adapter_fixtures.py",
    "tests/compat/packaging/test_packaging_smoke_fixtures.py",
    "assets/spark-app-icon-dark.png",
)
ALLOWED_SDIST_TEST_SOURCE_ENTRIES = frozenset(
    entry
    for entry in REQUIRED_SDIST_SOURCE_ENTRIES
    if entry.startswith("tests/")
)
ALLOWED_SDIST_TEST_SOURCE_DIRS = frozenset(
    parent
    for entry in ALLOWED_SDIST_TEST_SOURCE_ENTRIES
    for parent in Path(entry).parents
    if parent != Path(".")
)
FORBIDDEN_WHEEL_ENTRIES = (
    "spark/guides/attractor-spec.md",
    "spark/guides/spark-flow-extensions.md",
)
FORBIDDEN_ARTIFACT_DIR_NAMES = EXCLUDED_DIR_NAMES | {
    ".mypy_cache",
}
FORBIDDEN_SOURCE_TREE_PREFIXES = (
    Path("frontend") / "artifacts",
    Path("frontend") / "dist",
    Path("frontend") / "e2e",
    Path("frontend") / "node_modules",
    Path("frontend") / "playwright-report",
    Path("frontend") / "src" / "test",
    Path("frontend") / "test-results",
)
NATIVE_BINARY_MAGICS = (
    b"\x7fELF",
    b"MZ",
    b"\xca\xfe\xba\xbe",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
)
WHEEL_METADATA_SUFFIX = ".dist-info/WHEEL"
WHEEL_RECORD_SUFFIX = ".dist-info/RECORD"


@dataclass(frozen=True)
class BuildArtifacts:
    wheel: Path
    sdist: Path


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    _ensure_source_tree(repo_root)
    _ensure_package_versions_match(repo_root)
    _ensure_frontend_deps(repo_root)
    _run(["npm", "--prefix", "frontend", "run", "build"], cwd=repo_root)
    _build_rust_binaries(repo_root)

    with tempfile.TemporaryDirectory(prefix="spark-deliverable-stage-") as stage_str:
        stage_root = Path(stage_str)
        _copy_source_tree(repo_root, stage_root)
        _stage_packaged_ui(
            repo_root / "frontend" / "dist",
            stage_root / "src" / "spark" / "ui_dist",
        )
        _run(["uv", "build", "--sdist"], cwd=stage_root)
        _stage_rust_binaries(repo_root, stage_root)
        _run(["uv", "build", "--wheel"], cwd=stage_root)
        artifacts = _locate_artifacts(stage_root / "dist")
        artifacts = BuildArtifacts(
            wheel=_ensure_native_wheel_metadata(artifacts.wheel),
            sdist=artifacts.sdist,
        )
        _verify_artifact_contents(artifacts)
        output_dir = _resolve_output_dir(repo_root)
        _publish_artifacts(output_dir, artifacts)

    print(f"deliverable ready: {output_dir}")
    return 0


def _ensure_source_tree(repo_root: Path) -> None:
    missing = [
        relative_path
        for relative_path in REQUIRED_SOURCE_PATHS
        if not (repo_root / relative_path).exists()
    ]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"deliverable build source tree is missing required paths:\n{joined}")


def _ensure_package_versions_match(repo_root: Path) -> None:
    pyproject = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    cargo = tomllib.loads((repo_root / "Cargo.toml").read_text(encoding="utf-8"))
    python_version = pyproject.get("project", {}).get("version")
    rust_version = cargo.get("workspace", {}).get("package", {}).get("version")
    if not python_version or not rust_version:
        raise RuntimeError("package version metadata is missing from pyproject.toml or Cargo.toml")
    if python_version != rust_version:
        raise RuntimeError(
            "package version mismatch between pyproject.toml and Cargo.toml: "
            f"{python_version} != {rust_version}"
        )


def _ensure_frontend_deps(repo_root: Path) -> None:
    if all((repo_root / "frontend" / "node_modules" / ".bin" / binary).exists() for binary in FRONTEND_BINARIES):
        return
    _run(["npm", "--prefix", "frontend", "ci"], cwd=repo_root)


def _build_rust_binaries(repo_root: Path) -> None:
    for package_name, binary_name in RUST_BINARY_TARGETS:
        _run(
            [
                "cargo",
                "build",
                "--release",
                "-p",
                package_name,
                "--bin",
                binary_name,
            ],
            cwd=repo_root,
        )


def _copy_source_tree(repo_root: Path, stage_root: Path) -> None:
    for root_str, dir_names, file_names in os.walk(repo_root):
        root = Path(root_str)
        relative_root = root.relative_to(repo_root)
        dir_names[:] = [
            dir_name
            for dir_name in dir_names
            if not _is_excluded_source_path(
                _normalize_relative_path(relative_root / dir_name),
                is_dir=True,
            )
        ]
        for file_name in file_names:
            relative_path = _normalize_relative_path(relative_root / file_name)
            if _is_excluded_source_path(relative_path, is_dir=False):
                continue
            source = repo_root / relative_path
            target = stage_root / relative_path
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_symlink():
                if target.exists() or target.is_symlink():
                    target.unlink()
                target.symlink_to(os.readlink(source))
                continue
            shutil.copy2(source, target)


def _normalize_relative_path(path: Path) -> Path:
    return Path() if path == Path(".") else path


def _is_excluded_source_path(relative_path: Path, *, is_dir: bool) -> bool:
    if relative_path == Path():
        return False
    if is_dir and relative_path in ALLOWED_SDIST_TEST_SOURCE_DIRS:
        return False
    if not is_dir and relative_path.as_posix() in ALLOWED_SDIST_TEST_SOURCE_ENTRIES:
        return False
    if any(part in EXCLUDED_DIR_NAMES for part in relative_path.parts):
        return True
    if is_dir and relative_path in EXCLUDED_RELATIVE_DIRS:
        return True
    return not is_dir and relative_path.suffix in EXCLUDED_FILE_SUFFIXES


def _stage_packaged_ui(source_dist: Path, packaged_ui_dir: Path) -> None:
    index_path = source_dist / "index.html"
    if not index_path.exists():
        raise RuntimeError(f"frontend build did not produce index.html: {index_path}")
    if packaged_ui_dir.exists():
        shutil.rmtree(packaged_ui_dir)
    packaged_ui_dir.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source_dist, packaged_ui_dir)


def _stage_rust_binaries(repo_root: Path, stage_root: Path) -> None:
    packaged_bin_dir = stage_root / "src" / "spark" / "bin"
    if packaged_bin_dir.exists():
        shutil.rmtree(packaged_bin_dir)
    packaged_bin_dir.mkdir(parents=True, exist_ok=True)

    for _package_name, binary_name in RUST_BINARY_TARGETS:
        source = repo_root / "target" / "release" / _platform_binary_name(binary_name)
        if not source.is_file():
            raise RuntimeError(f"Rust release binary is missing: {source}")
        if not _is_executable(source):
            raise RuntimeError(f"Rust release binary is not executable: {source}")
        target = packaged_bin_dir / source.name
        shutil.copy2(source, target)
        source_mode = stat.S_IMODE(source.stat().st_mode)
        target.chmod(source_mode)
        if not _is_executable(target):
            raise RuntimeError(f"staged Rust binary is not executable: {target}")


def _platform_binary_name(binary_name: str) -> str:
    return f"{binary_name}.exe" if os.name == "nt" else binary_name


def _is_executable(path: Path) -> bool:
    return path.is_file() and os.access(path, os.X_OK)


def _locate_artifacts(dist_dir: Path) -> BuildArtifacts:
    wheels = sorted(dist_dir.glob("spark-*.whl"))
    sdists = sorted(dist_dir.glob("spark-*.tar.gz"))
    if len(wheels) != 1 or len(sdists) != 1:
        raise RuntimeError(f"expected exactly one spark wheel and one spark sdist in {dist_dir}")
    return BuildArtifacts(wheel=wheels[0], sdist=sdists[0])


def _verify_artifact_contents(artifacts: BuildArtifacts) -> None:
    _verify_wheel_contents(artifacts.wheel)
    _verify_sdist_contents(artifacts.sdist)
    _verify_artifact_exclusions(artifacts.wheel, _zip_entries(artifacts.wheel))
    _verify_artifact_exclusions(artifacts.sdist, _tar_entries(artifacts.sdist))


def _verify_wheel_contents(wheel_path: Path) -> None:
    with zipfile.ZipFile(wheel_path) as wheel_file:
        names = set(wheel_file.namelist())
        _verify_native_binary_entries(wheel_file, names)
        _verify_native_wheel_metadata(wheel_path, wheel_file, names)
    missing = [entry for entry in REQUIRED_WHEEL_ENTRIES if entry not in names]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"wheel is missing required packaged assets:\n{joined}")
    ui_assets = [
        entry
        for entry in names
        if entry.startswith("spark/ui_dist/assets/")
        and not entry.endswith("/")
    ]
    if not any(entry.endswith(".js") for entry in ui_assets):
        raise RuntimeError("wheel is missing built frontend JavaScript assets")
    if not any(entry.endswith(".css") for entry in ui_assets):
        raise RuntimeError("wheel is missing built frontend CSS assets")
    present_forbidden = [entry for entry in FORBIDDEN_WHEEL_ENTRIES if entry in names]
    if present_forbidden:
        joined = "\n".join(present_forbidden)
        raise RuntimeError(f"wheel unexpectedly contains removed packaged specs:\n{joined}")


def _verify_sdist_contents(sdist_path: Path) -> None:
    relative_entries = _sdist_relative_entries(_tar_entries(sdist_path))
    missing = [
        entry
        for entry in REQUIRED_SDIST_SOURCE_ENTRIES
        if entry not in relative_entries
    ]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"sdist is missing required Rust rewrite source entries:\n{joined}")
    staged_binaries = [
        entry
        for entry in FORBIDDEN_SDIST_BINARY_ENTRIES
        if entry in relative_entries
    ]
    if staged_binaries:
        joined = "\n".join(staged_binaries)
        raise RuntimeError(f"sdist unexpectedly contains staged Rust binaries:\n{joined}")


def _sdist_relative_entries(entries: set[str]) -> set[str]:
    relative_entries: set[str] = set()
    for entry in entries:
        parts = Path(entry).parts
        if len(parts) > 1 and parts[0].startswith("spark-"):
            relative_entries.add(Path(*parts[1:]).as_posix())
        else:
            relative_entries.add(Path(entry).as_posix())
    return relative_entries


def _ensure_native_wheel_metadata(wheel_path: Path) -> Path:
    native_tag = _native_wheel_tag()
    with zipfile.ZipFile(wheel_path) as wheel_file:
        names = set(wheel_file.namelist())
        _verify_native_binary_entries(wheel_file, names)
        wheel_metadata_name = _single_wheel_entry(names, WHEEL_METADATA_SUFFIX)
        wheel_metadata = wheel_file.read(wheel_metadata_name).decode("utf-8")
    return _rewrite_wheel_metadata(
        wheel_path,
        wheel_metadata_name=wheel_metadata_name,
        wheel_metadata=_updated_wheel_metadata(wheel_metadata, native_tag),
        updated_name=_retagged_wheel_name(wheel_path.name, native_tag),
    )


def _verify_native_binary_entries(
    wheel_file: zipfile.ZipFile,
    names: set[str],
) -> None:
    missing = [entry for entry in REQUIRED_RUST_BINARY_ENTRIES if entry not in names]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"wheel is missing required Rust binaries:\n{joined}")
    non_native = [
        entry
        for entry in REQUIRED_RUST_BINARY_ENTRIES
        if not _is_native_binary_payload(wheel_file.read(entry))
    ]
    if non_native:
        joined = "\n".join(non_native)
        raise RuntimeError(f"wheel Rust binary entries are not native executables:\n{joined}")


def _verify_native_wheel_metadata(
    wheel_path: Path,
    wheel_file: zipfile.ZipFile,
    names: set[str],
) -> None:
    wheel_metadata_name = _single_wheel_entry(names, WHEEL_METADATA_SUFFIX)
    metadata = _wheel_metadata_fields(
        wheel_file.read(wheel_metadata_name).decode("utf-8")
    )
    native_tag = _native_wheel_tag()
    root_is_purelib = str(metadata.get("root_is_purelib", "")).lower()
    tags = set(metadata.get("tags", ()))
    if root_is_purelib != "false":
        raise RuntimeError(
            "wheel contains native Rust binaries but WHEEL metadata does not set "
            "Root-Is-Purelib: false"
        )
    if "py3-none-any" in tags or native_tag not in tags:
        raise RuntimeError(
            "wheel contains native Rust binaries but WHEEL metadata does not use "
            f"the platform tag {native_tag}"
        )
    if not wheel_path.name.endswith(f"-{native_tag}.whl"):
        raise RuntimeError(
            "wheel contains native Rust binaries but the wheel filename does not "
            f"use the platform tag {native_tag}: {wheel_path.name}"
        )


def _is_native_binary_payload(payload: bytes) -> bool:
    return any(payload.startswith(magic) for magic in NATIVE_BINARY_MAGICS)


def _native_wheel_tag() -> str:
    return f"py3-none-{_wheel_platform_tag()}"


def _wheel_platform_tag() -> str:
    return sysconfig.get_platform().replace("-", "_").replace(".", "_")


def _updated_wheel_metadata(metadata: str, native_tag: str) -> str:
    lines = metadata.splitlines()
    updated: list[str] = []
    wrote_root = False
    wrote_tag = False
    for line in lines:
        if line.startswith("Root-Is-Purelib:"):
            updated.append("Root-Is-Purelib: false")
            wrote_root = True
        elif line.startswith("Tag:"):
            if not wrote_tag:
                updated.append(f"Tag: {native_tag}")
                wrote_tag = True
        else:
            updated.append(line)
    if not wrote_root:
        updated.append("Root-Is-Purelib: false")
    if not wrote_tag:
        updated.append(f"Tag: {native_tag}")
    return "\n".join(updated) + "\n"


def _wheel_metadata_fields(metadata: str) -> dict[str, str | tuple[str, ...]]:
    tags: list[str] = []
    root_is_purelib = ""
    for line in metadata.splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        normalized_key = key.strip().lower()
        normalized_value = value.strip()
        if normalized_key == "root-is-purelib":
            root_is_purelib = normalized_value
        elif normalized_key == "tag":
            tags.append(normalized_value)
    return {
        "root_is_purelib": root_is_purelib,
        "tags": tuple(tags),
    }


def _single_wheel_entry(names: set[str], suffix: str) -> str:
    matches = sorted(name for name in names if name.endswith(suffix))
    if len(matches) != 1:
        raise RuntimeError(f"expected exactly one wheel metadata entry ending with {suffix}")
    return matches[0]


def _retagged_wheel_name(wheel_name: str, native_tag: str) -> str:
    if not wheel_name.endswith(".whl"):
        raise RuntimeError(f"expected a wheel filename, got {wheel_name}")
    stem_parts = wheel_name[:-4].split("-")
    if len(stem_parts) < 5:
        raise RuntimeError(f"cannot retag malformed wheel filename: {wheel_name}")
    return "-".join([*stem_parts[:-3], *native_tag.split("-")]) + ".whl"


def _rewrite_wheel_metadata(
    wheel_path: Path,
    *,
    wheel_metadata_name: str,
    wheel_metadata: str,
    updated_name: str,
) -> Path:
    with zipfile.ZipFile(wheel_path) as source:
        infos = source.infolist()
        payloads = {
            info.filename: b"" if info.is_dir() else source.read(info.filename)
            for info in infos
        }
    record_name = _single_wheel_entry(set(payloads), WHEEL_RECORD_SUFFIX)
    payloads[wheel_metadata_name] = wheel_metadata.encode("utf-8")
    payloads[record_name] = _wheel_record_payload(payloads, record_name)

    target_path = wheel_path.with_name(updated_name)
    temp_path = target_path.with_name(f"{target_path.name}.tmp")
    if temp_path.exists():
        temp_path.unlink()
    with zipfile.ZipFile(temp_path, "w", compression=zipfile.ZIP_DEFLATED) as target:
        for info in infos:
            updated_info = zipfile.ZipInfo(info.filename, info.date_time)
            updated_info.comment = info.comment
            updated_info.extra = info.extra
            updated_info.internal_attr = info.internal_attr
            updated_info.external_attr = info.external_attr
            updated_info.create_system = info.create_system
            updated_info.compress_type = info.compress_type
            target.writestr(updated_info, payloads[info.filename])
    temp_path.replace(target_path)
    if target_path != wheel_path:
        wheel_path.unlink()
    return target_path


def _wheel_record_payload(payloads: dict[str, bytes], record_name: str) -> bytes:
    output = io.StringIO(newline="")
    writer = csv.writer(output, lineterminator="\n")
    for name in sorted(payloads):
        if name == record_name or name.endswith("/"):
            continue
        payload = payloads[name]
        digest = base64.urlsafe_b64encode(hashlib.sha256(payload).digest()).rstrip(
            b"="
        ).decode("ascii")
        writer.writerow([name, f"sha256={digest}", str(len(payload))])
    writer.writerow([record_name, "", ""])
    return output.getvalue().encode("utf-8")


def _verify_artifact_exclusions(artifact_path: Path, entries: set[str]) -> None:
    forbidden = sorted(
        entry for entry in entries if _is_forbidden_artifact_entry(artifact_path, entry)
    )
    if forbidden:
        joined = "\n".join(forbidden[:25])
        raise RuntimeError(f"{artifact_path.name} contains forbidden generated or test-only paths:\n{joined}")


def _zip_entries(path: Path) -> set[str]:
    with zipfile.ZipFile(path) as archive:
        return set(archive.namelist())


def _tar_entries(path: Path) -> set[str]:
    with tarfile.open(path) as archive:
        return set(archive.getnames())


def _is_forbidden_artifact_entry(artifact_path: Path, entry: str) -> bool:
    if artifact_path.name.endswith(".tar.gz"):
        relative_entry = _artifact_relative_entry(entry)
        if (
            relative_entry in ALLOWED_SDIST_TEST_SOURCE_ENTRIES
            or Path(relative_entry) in ALLOWED_SDIST_TEST_SOURCE_DIRS
        ):
            return False
    relative = Path(entry)
    parts = relative.parts
    if any(part in FORBIDDEN_ARTIFACT_DIR_NAMES for part in parts):
        return True
    if relative.suffix in EXCLUDED_FILE_SUFFIXES:
        return True
    for index in range(len(parts)):
        suffix = Path(*parts[index:])
        if any(suffix == prefix or prefix in suffix.parents for prefix in FORBIDDEN_SOURCE_TREE_PREFIXES):
            return True
    return False


def _artifact_relative_entry(entry: str) -> str:
    parts = Path(entry).parts
    if len(parts) > 1 and parts[0].startswith("spark-"):
        return Path(*parts[1:]).as_posix()
    return Path(entry).as_posix()


def _publish_artifacts(repo_dist: Path, artifacts: BuildArtifacts) -> None:
    repo_dist.mkdir(parents=True, exist_ok=True)
    for old_artifact in list(repo_dist.glob("spark-*.whl")) + list(
        repo_dist.glob("spark-*.tar.gz")
    ):
        old_artifact.unlink()
    copied_artifacts = [
        shutil.copy2(artifacts.wheel, repo_dist / artifacts.wheel.name),
        shutil.copy2(artifacts.sdist, repo_dist / artifacts.sdist.name),
    ]
    _chown_artifacts(copied_artifacts)


def _resolve_output_dir(repo_root: Path) -> Path:
    output_dir = os.environ.get("SPARK_DELIVERABLE_OUT")
    if output_dir:
        return Path(output_dir)
    return repo_root / "dist"


def _chown_artifacts(artifact_paths: list[str | Path]) -> None:
    raw_uid = os.environ.get("HOST_UID")
    raw_gid = os.environ.get("HOST_GID")
    if raw_uid is None and raw_gid is None:
        return
    if raw_uid is None or raw_gid is None:
        raise RuntimeError("HOST_UID and HOST_GID must be provided together")
    try:
        uid = int(raw_uid)
        gid = int(raw_gid)
    except ValueError as exc:
        raise RuntimeError("HOST_UID and HOST_GID must be integer values") from exc
    for artifact_path in artifact_paths:
        os.chown(artifact_path, uid, gid)


def _run(command: list[str], *, cwd: Path, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command))
    return subprocess.run(
        command,
        cwd=cwd,
        check=True,
        text=True,
        capture_output=capture_output,
    )


if __name__ == "__main__":
    raise SystemExit(main())
