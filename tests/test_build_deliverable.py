from __future__ import annotations

import io
import tarfile
import zipfile
from pathlib import Path

import pytest

from scripts import build_deliverable


def _write(path: Path, content: str = "") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _native_binary_payload() -> bytes:
    return b"\x7fELF\x02\x01\x01\x00" + (b"\x00" * 24)


def _write_required_wheel(
    wheel_path: Path,
    *,
    root_is_purelib: bool = False,
    tag: str | None = None,
    extra_entries: dict[str, bytes | str] | None = None,
    entry_overrides: dict[str, bytes | str] | None = None,
) -> None:
    native_tag = tag or build_deliverable._native_wheel_tag()
    root_value = "true" if root_is_purelib else "false"
    overrides = entry_overrides or {}
    with zipfile.ZipFile(wheel_path, "w") as archive:
        for entry in build_deliverable.REQUIRED_WHEEL_ENTRIES:
            payload: bytes | str = (
                _native_binary_payload()
                if entry in build_deliverable.REQUIRED_RUST_BINARY_ENTRIES
                else "ok"
            )
            payload = overrides.get(entry, payload)
            archive.writestr(entry, payload)
        archive.writestr("spark/ui_dist/assets/index-test.js", "ok")
        archive.writestr("spark/ui_dist/assets/index-test.css", "ok")
        archive.writestr(
            "spark-0.1.0.dist-info/WHEEL",
            "\n".join(
                [
                    "Wheel-Version: 1.0",
                    "Generator: test",
                    f"Root-Is-Purelib: {root_value}",
                    f"Tag: {native_tag}",
                    "",
                ]
            ),
        )
        archive.writestr("spark-0.1.0.dist-info/RECORD", "")
        for entry, payload in (extra_entries or {}).items():
            archive.writestr(entry, payload)


def _write_required_sdist(
    sdist_path: Path,
    *,
    extra_entries: dict[str, bytes | str] | None = None,
    omit_entries: set[str] | None = None,
) -> None:
    omitted = omit_entries or set()
    with tarfile.open(sdist_path, "w:gz") as archive:
        for entry in build_deliverable.REQUIRED_SDIST_SOURCE_ENTRIES:
            if entry in omitted:
                continue
            _add_tar_payload(archive, f"spark-0.1.0/{entry}", "ok")
        for entry, payload in (extra_entries or {}).items():
            _add_tar_payload(archive, entry, payload)


def _add_tar_payload(
    archive: tarfile.TarFile,
    name: str,
    payload: bytes | str,
) -> None:
    data = payload.encode("utf-8") if isinstance(payload, str) else payload
    info = tarfile.TarInfo(name)
    info.size = len(data)
    archive.addfile(info, io.BytesIO(data))


def test_copy_source_tree_copies_filesystem_sources_without_generated_outputs(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "source"
    stage_root = tmp_path / "stage"
    _write(source_root / "pyproject.toml")
    _write(source_root / "Cargo.toml")
    _write(source_root / "Dockerfile")
    _write(source_root / "Dockerfile.wheel")
    _write(source_root / "compose.package.yaml")
    _write(source_root / "scripts" / "build_deliverable.py")
    _write(source_root / "scripts" / "package-entrypoint.sh")
    _write(source_root / "scripts" / "run-docker.sh")
    _write(source_root / "src" / "spark" / "__init__.py")
    _write(source_root / "frontend" / "package-lock.json")
    _write(source_root / "frontend" / "src" / "main.ts")
    _write(source_root / "frontend" / "src" / "test" / "setup.ts")
    _write(source_root / "frontend" / "src" / "features" / "__tests__" / "panel.test.ts")
    _write(source_root / "frontend" / "e2e" / "smoke" / "app.spec.ts")
    _write(source_root / "frontend" / "artifacts" / "ui-smoke" / "screen.png")
    _write(source_root / "frontend" / "test-results" / "result.json")
    _write(source_root / "frontend" / "playwright-report" / "index.html")
    _write(source_root / ".git" / "HEAD")
    _write(source_root / ".spark" / "state.json")
    _write(source_root / ".venv" / "pyvenv.cfg")
    _write(source_root / "dist" / "spark-0.1.0.whl")
    _write(source_root / "target" / "release" / "spark")
    _write(source_root / "node_modules" / "cache")
    _write(source_root / "tests" / "test_only.py")
    _write(source_root / "tests" / "compat" / "providers" / "test_unified_llm_adapter_fixtures.py")
    _write(source_root / "tests" / "compat" / "packaging" / "test_packaging_smoke_fixtures.py")
    _write(source_root / "tests" / "compat" / "api" / "test_http_route_fixtures.py")
    _write(source_root / "frontend" / "node_modules" / ".bin" / "vite")
    _write(source_root / "frontend" / "dist" / "index.html")
    _write(source_root / "src" / "spark" / "__pycache__" / "module.cpython-311.pyc")

    build_deliverable._copy_source_tree(source_root, stage_root)

    assert (stage_root / "pyproject.toml").exists()
    assert (stage_root / "Cargo.toml").exists()
    assert (stage_root / "Dockerfile").exists()
    assert (stage_root / "Dockerfile.wheel").exists()
    assert (stage_root / "compose.package.yaml").exists()
    assert (stage_root / "scripts" / "build_deliverable.py").exists()
    assert (stage_root / "scripts" / "package-entrypoint.sh").exists()
    assert (stage_root / "scripts" / "run-docker.sh").exists()
    assert (stage_root / "src" / "spark" / "__init__.py").exists()
    assert (stage_root / "frontend" / "package-lock.json").exists()
    assert (stage_root / "frontend" / "src" / "main.ts").exists()
    assert not (stage_root / "frontend" / "src" / "test").exists()
    assert not (stage_root / "frontend" / "src" / "features" / "__tests__").exists()
    assert not (stage_root / "frontend" / "e2e").exists()
    assert not (stage_root / "frontend" / "artifacts").exists()
    assert not (stage_root / "frontend" / "test-results").exists()
    assert not (stage_root / "frontend" / "playwright-report").exists()
    assert not (stage_root / ".git").exists()
    assert not (stage_root / ".spark").exists()
    assert not (stage_root / ".venv").exists()
    assert not (stage_root / "dist").exists()
    assert not (stage_root / "target").exists()
    assert not (stage_root / "node_modules").exists()
    assert not (stage_root / "tests" / "test_only.py").exists()
    assert (stage_root / "tests" / "compat" / "providers" / "test_unified_llm_adapter_fixtures.py").exists()
    assert (stage_root / "tests" / "compat" / "packaging" / "test_packaging_smoke_fixtures.py").exists()
    assert not (stage_root / "tests" / "compat" / "api").exists()
    assert not (stage_root / "frontend" / "node_modules").exists()
    assert not (stage_root / "frontend" / "dist").exists()
    assert not (stage_root / "src" / "spark" / "__pycache__").exists()


def test_ensure_source_tree_reports_missing_required_paths(tmp_path: Path) -> None:
    with pytest.raises(RuntimeError, match="pyproject.toml"):
        build_deliverable._ensure_source_tree(tmp_path)


def test_ensure_package_versions_match_rejects_mismatch(tmp_path: Path) -> None:
    _write(
        tmp_path / "pyproject.toml",
        "[project]\nname = \"spark\"\nversion = \"0.1.0\"\n",
    )
    _write(
        tmp_path / "Cargo.toml",
        "[workspace]\n[workspace.package]\nversion = \"0.2.0\"\n",
    )

    with pytest.raises(RuntimeError, match="package version mismatch"):
        build_deliverable._ensure_package_versions_match(tmp_path)


def test_stage_rust_binaries_copies_release_outputs_with_executable_bits(
    tmp_path: Path,
) -> None:
    repo_root = tmp_path / "repo"
    stage_root = tmp_path / "stage"
    release_dir = repo_root / "target" / "release"
    spark = release_dir / "spark"
    spark_server = release_dir / "spark-server"
    _write(spark, "#!/bin/sh\n")
    _write(spark_server, "#!/bin/sh\n")
    spark.chmod(0o755)
    spark_server.chmod(0o755)

    build_deliverable._stage_rust_binaries(repo_root, stage_root)

    staged_spark = stage_root / "src" / "spark" / "bin" / "spark"
    staged_server = stage_root / "src" / "spark" / "bin" / "spark-server"
    assert staged_spark.read_text(encoding="utf-8") == "#!/bin/sh\n"
    assert staged_server.read_text(encoding="utf-8") == "#!/bin/sh\n"
    assert staged_spark.stat().st_mode & 0o111
    assert staged_server.stat().st_mode & 0o111


def test_verify_artifact_contents_rejects_generated_and_test_only_entries(
    tmp_path: Path,
) -> None:
    wheel = tmp_path / f"spark-0.1.0-{build_deliverable._native_wheel_tag()}.whl"
    sdist = tmp_path / "spark-0.1.0.tar.gz"
    _write_required_wheel(
        wheel,
        extra_entries={"spark/__pycache__/module.pyc": "compiled"},
    )
    _write_required_sdist(
        sdist,
        extra_entries={"spark-0.1.0/tests/generated.pyc": "compiled"},
    )

    with pytest.raises(RuntimeError, match="forbidden generated or test-only paths"):
        build_deliverable._verify_artifact_contents(
            build_deliverable.BuildArtifacts(wheel=wheel, sdist=sdist)
        )


def test_verify_artifact_contents_rejects_sdist_missing_rust_sources(
    tmp_path: Path,
) -> None:
    wheel = tmp_path / f"spark-0.1.0-{build_deliverable._native_wheel_tag()}.whl"
    sdist = tmp_path / "spark-0.1.0.tar.gz"
    _write_required_wheel(wheel)
    _write_required_sdist(sdist, omit_entries={"Cargo.lock"})

    with pytest.raises(RuntimeError, match="sdist is missing required Rust rewrite source entries"):
        build_deliverable._verify_artifact_contents(
            build_deliverable.BuildArtifacts(wheel=wheel, sdist=sdist)
        )


def test_ensure_native_wheel_metadata_retags_universal_wheel(tmp_path: Path) -> None:
    wheel = tmp_path / "spark-0.1.0-py3-none-any.whl"
    _write_required_wheel(wheel, root_is_purelib=True, tag="py3-none-any")

    updated_wheel = build_deliverable._ensure_native_wheel_metadata(wheel)

    native_tag = build_deliverable._native_wheel_tag()
    assert updated_wheel.name == f"spark-0.1.0-{native_tag}.whl"
    assert updated_wheel.exists()
    assert not wheel.exists()
    with zipfile.ZipFile(updated_wheel) as archive:
        wheel_metadata = archive.read("spark-0.1.0.dist-info/WHEEL").decode("utf-8")
        record = archive.read("spark-0.1.0.dist-info/RECORD").decode("utf-8")
    assert "Root-Is-Purelib: false" in wheel_metadata
    assert f"Tag: {native_tag}" in wheel_metadata
    assert "Tag: py3-none-any" not in wheel_metadata
    assert "spark-0.1.0.dist-info/WHEEL,sha256=" in record
    build_deliverable._verify_wheel_contents(updated_wheel)


def test_verify_wheel_contents_rejects_pure_metadata_for_native_binary(
    tmp_path: Path,
) -> None:
    wheel = tmp_path / "spark-0.1.0-py3-none-any.whl"
    _write_required_wheel(wheel, root_is_purelib=True, tag="py3-none-any")

    with pytest.raises(RuntimeError, match="Root-Is-Purelib"):
        build_deliverable._verify_wheel_contents(wheel)


def test_verify_wheel_contents_rejects_non_native_rust_binary(
    tmp_path: Path,
) -> None:
    wheel = tmp_path / f"spark-0.1.0-{build_deliverable._native_wheel_tag()}.whl"
    _write_required_wheel(
        wheel,
        entry_overrides={"spark/bin/spark": b"#!/bin/sh\n"},
    )

    with pytest.raises(RuntimeError, match="not native executables"):
        build_deliverable._verify_wheel_contents(wheel)


def test_publish_artifacts_replaces_outputs_and_applies_host_ownership(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    built_dist = tmp_path / "built-dist"
    output_dir = tmp_path / "output"
    wheel = built_dist / "spark-0.1.0-py3-none-any.whl"
    sdist = built_dist / "spark-0.1.0.tar.gz"
    _write(wheel, "wheel")
    _write(sdist, "sdist")
    _write(output_dir / "spark-old.whl", "old wheel")
    _write(output_dir / "spark-old.tar.gz", "old sdist")
    chowned: list[tuple[Path, int, int]] = []

    monkeypatch.setenv("HOST_UID", "123")
    monkeypatch.setenv("HOST_GID", "456")
    monkeypatch.setattr(
        build_deliverable.os,
        "chown",
        lambda path, uid, gid: chowned.append((Path(path), uid, gid)),
    )

    build_deliverable._publish_artifacts(
        output_dir,
        build_deliverable.BuildArtifacts(wheel=wheel, sdist=sdist),
    )

    assert sorted(path.name for path in output_dir.iterdir()) == [
        "spark-0.1.0-py3-none-any.whl",
        "spark-0.1.0.tar.gz",
    ]
    assert chowned == [
        (output_dir / "spark-0.1.0-py3-none-any.whl", 123, 456),
        (output_dir / "spark-0.1.0.tar.gz", 123, 456),
    ]
