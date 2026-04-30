from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Mapping, Sequence

import spark.starter_assets as starter_assets

SERVICE_UNIT_NAME = "spark.service"


def _running_from_source_checkout(project_root: Path) -> bool:
    return (
        (project_root / ".git").exists()
        or (
            (project_root / "pyproject.toml").is_file()
            and (project_root / "src" / "spark" / "flows").is_dir()
            and (project_root / "frontend").is_dir()
        )
    )


def _require_explicit_dev_home(
    *,
    command: str,
    data_dir: Path | None,
    env: Mapping[str, str] | None = None,
) -> None:
    env_map = env if env is not None else os.environ
    project_root = Path(__file__).resolve().parents[2]
    if not _running_from_source_checkout(project_root):
        return
    if data_dir is not None or env_map.get("SPARK_HOME"):
        return
    raise RuntimeError(
        "\n".join(
            [
                f"Refusing to use default runtime home ~/.spark from a source checkout at {project_root}.",
                "",
                "The default home is reserved for the installed or stable Spark instance.",
                f"Run the source checkout with an explicit dev home before `{command}`, for example:",
                "",
                "  SPARK_HOME=~/.spark-dev uv run spark-server init",
                "  SPARK_HOME=~/.spark-dev uv run spark-server serve --reload --port 8010",
            ]
        )
    )


def _build_runtime_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="spark-server",
        description="Spark operator CLI",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    subparsers = parser.add_subparsers(dest="command")

    serve = subparsers.add_parser(
        "serve",
        help="Start the Spark API server",
    )
    serve.add_argument("--host", default="127.0.0.1", help="Bind host (default: 127.0.0.1)")
    serve.add_argument("--port", type=int, default=8000, help="Bind port (default: 8000)")
    serve.add_argument("--reload", action="store_true", help="Enable auto-reload (development only)")
    serve.add_argument("--data-dir", type=Path, default=None, help="Runtime data directory root")
    serve.add_argument("--flows-dir", type=Path, default=None, help="Flow storage directory")
    serve.add_argument("--ui-dir", type=Path, default=None, help="Built UI directory (contains index.html)")

    init = subparsers.add_parser(
        "init",
        help="Initialize Spark runtime directories and seed packaged flows",
    )
    init.add_argument("--data-dir", type=Path, default=None, help="Runtime data directory root")
    init.add_argument("--flows-dir", type=Path, default=None, help="Flow storage directory")
    init.add_argument("--force", action="store_true", help="Overwrite existing packaged flows")

    service = subparsers.add_parser(
        "service",
        help="Manage the installed Spark user service",
    )
    service_commands = service.add_subparsers(dest="service_command")

    service_install = service_commands.add_parser(
        "install",
        help="Install or update a user-level systemd service and start it",
    )
    service_install.add_argument("--host", default="0.0.0.0", help="Bind host (default: 0.0.0.0)")
    service_install.add_argument("--port", type=int, default=8000, help="Bind port (default: 8000)")
    service_install.add_argument("--data-dir", type=Path, default=None, help="Runtime data directory root")
    service_install.add_argument("--flows-dir", type=Path, default=None, help="Flow storage directory")
    service_install.add_argument("--ui-dir", type=Path, default=None, help="Built UI directory (contains index.html)")

    service_commands.add_parser(
        "remove",
        help="Stop and remove the user-level systemd service",
    )
    service_commands.add_parser(
        "status",
        help="Show the current user-level systemd service status",
    )

    worker = subparsers.add_parser(
        "worker",
        help=argparse.SUPPRESS,
    )
    worker_commands = worker.add_subparsers(dest="worker_command")
    worker_commands.add_parser("run-node", help=argparse.SUPPRESS)

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _build_runtime_parser()
    args = parser.parse_args(argv)

    if not getattr(args, "command", None):
        parser.print_help()
        return 0

    if args.command == "serve":
        return _run_serve(args)
    if args.command == "init":
        return _run_init(args)
    if args.command == "service":
        return _run_service_command(parser, args)
    if args.command == "worker":
        return _run_worker_command(parser, args)
    parser.error(f"Unknown command: {args.command}")
    return 2


def _run_worker_command(parser: argparse.ArgumentParser, args: argparse.Namespace) -> int:
    if args.worker_command == "run-node":
        from attractor.handlers.execution_container import run_worker_node

        return run_worker_node()
    parser.error("worker requires a subcommand")
    return 2


def _run_service_command(parser: argparse.ArgumentParser, args: argparse.Namespace) -> int:
    if args.service_command == "install":
        return _run_service_install(args)
    if args.service_command == "remove":
        return _run_service_remove()
    if args.service_command == "status":
        return _run_service_status()
    parser.error("service requires a subcommand")
    return 2


def _run_serve(args: argparse.Namespace) -> int:
    try:
        _require_explicit_dev_home(command="spark-server serve", data_dir=args.data_dir)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    import uvicorn

    import spark.app as product_app
    from spark.settings import ENV_FLOWS_DIR, ENV_HOME_DIR, ENV_UI_DIR

    def _set_path_env(name: str, value: Path | None) -> None:
        if value is None:
            return
        os.environ[name] = str(value.expanduser().resolve(strict=False))

    product_app.configure_settings(
        data_dir=args.data_dir,
        flows_dir=args.flows_dir,
        ui_dir=args.ui_dir,
    )
    product_app.validate_settings()
    _set_path_env(ENV_HOME_DIR, args.data_dir)
    _set_path_env(ENV_FLOWS_DIR, args.flows_dir)
    _set_path_env(ENV_UI_DIR, args.ui_dir)

    uvicorn.run(
        "spark.app:app" if args.reload else product_app.app,
        host=args.host,
        port=args.port,
        reload=args.reload,
    )
    return 0


def _run_init(args: argparse.Namespace) -> int:
    try:
        _require_explicit_dev_home(command="spark-server init", data_dir=args.data_dir)
        settings, result = _initialize_runtime(
            data_dir=args.data_dir,
            flows_dir=args.flows_dir,
            force=args.force,
        )
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    print(f"Initialized Spark at {settings.data_dir}")
    print(f"Seeded flows: {result.flows_dir}")
    print(
        "created={created} updated={updated} skipped={skipped}".format(
            created=len(result.created),
            updated=len(result.updated),
            skipped=len(result.skipped),
        )
    )
    return 0


def _run_service_install(args: argparse.Namespace) -> int:
    try:
        _require_explicit_dev_home(command="spark-server service install", data_dir=args.data_dir)
        systemctl = _require_systemd_user_support()
        settings, result = _initialize_runtime(
            data_dir=args.data_dir,
            flows_dir=args.flows_dir,
            ui_dir=args.ui_dir,
            force=False,
        )
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    unit_path = _service_unit_path()
    try:
        unit_path.parent.mkdir(parents=True, exist_ok=True)
        unit_path.write_text(_build_service_unit(settings=settings, host=args.host, port=args.port), encoding="utf-8")
    except OSError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    try:
        _run_systemctl(systemctl, "daemon-reload")
        _run_systemctl(systemctl, "enable", SERVICE_UNIT_NAME)
        _run_systemctl(systemctl, "restart", SERVICE_UNIT_NAME)
    except subprocess.CalledProcessError as exc:
        _print_systemctl_failure(exc)
        return 1

    print(f"Installed Spark user service: {unit_path}")
    print(f"Service name: {SERVICE_UNIT_NAME}")
    print(f"Listening on http://{args.host}:{args.port}")
    print(f"Initialized Spark at {settings.data_dir}")
    print(f"Seeded flows: {result.flows_dir}")
    print(
        "created={created} updated={updated} skipped={skipped}".format(
            created=len(result.created),
            updated=len(result.updated),
            skipped=len(result.skipped),
        )
    )
    return 0


def _run_service_remove() -> int:
    try:
        systemctl = _require_systemd_user_support()
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    unit_path = _service_unit_path()
    _run_systemctl(systemctl, "disable", "--now", SERVICE_UNIT_NAME, check=False)
    if unit_path.exists():
        unit_path.unlink()

    try:
        _run_systemctl(systemctl, "daemon-reload")
    except subprocess.CalledProcessError as exc:
        _print_systemctl_failure(exc)
        return 1

    _run_systemctl(systemctl, "reset-failed", SERVICE_UNIT_NAME, check=False)
    print(f"Removed Spark user service: {unit_path}")
    return 0


def _run_service_status() -> int:
    try:
        systemctl = _require_systemd_user_support()
        completed = _run_systemctl(
            systemctl,
            "--no-pager",
            "--full",
            "status",
            SERVICE_UNIT_NAME,
            check=False,
        )
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    if completed.stdout:
        print(completed.stdout, end="")
    if completed.stderr:
        print(completed.stderr, end="", file=sys.stderr)
    return completed.returncode


def _initialize_runtime(
    *,
    data_dir: Path | None,
    flows_dir: Path | None,
    ui_dir: Path | None = None,
    force: bool,
):
    from spark.settings import resolve_settings, validate_settings

    settings = resolve_settings(
        data_dir=data_dir,
        flows_dir=flows_dir,
        ui_dir=ui_dir,
    )
    validate_settings(settings)
    result = starter_assets.seed_starter_flows(
        settings.flows_dir,
        force=force,
        project_root=settings.project_root,
    )
    return settings, result


def _require_systemd_user_support() -> str:
    if sys.platform != "linux":
        raise RuntimeError("Spark service management currently supports Linux user services via systemd only.")
    systemctl = shutil.which("systemctl")
    if not systemctl:
        raise RuntimeError("systemctl is not available on PATH.")
    return systemctl


def _service_unit_path(env: Mapping[str, str] | None = None) -> Path:
    env_map = env if env is not None else os.environ
    xdg_config_home = env_map.get("XDG_CONFIG_HOME")
    config_home = Path(xdg_config_home).expanduser() if xdg_config_home else Path.home() / ".config"
    return config_home / "systemd" / "user" / SERVICE_UNIT_NAME


def _build_service_unit(*, settings, host: str, port: int) -> str:
    provider_env_file = settings.config_dir / "provider.env"
    service_environment = [
        _quote_systemd_arg("PYTHONUNBUFFERED=1"),
        _quote_systemd_arg(f"PATH={os.environ.get('PATH', '')}"),
        _quote_systemd_arg(f"SPARK_HOME={settings.data_dir}"),
        _quote_systemd_arg(f"SPARK_FLOWS_DIR={settings.flows_dir}"),
    ]
    if settings.ui_dir is not None:
        service_environment.append(_quote_systemd_arg(f"SPARK_UI_DIR={settings.ui_dir}"))

    exec_args = [
        sys.executable,
        "-m",
        "spark.server_cli",
        "serve",
        "--host",
        host,
        "--port",
        str(port),
        "--data-dir",
        str(settings.data_dir),
        "--flows-dir",
        str(settings.flows_dir),
    ]
    if settings.ui_dir is not None:
        exec_args.extend(["--ui-dir", str(settings.ui_dir)])

    exec_start = " ".join(_quote_systemd_arg(arg) for arg in exec_args)
    return "\n".join(
        [
            "[Unit]",
            "Description=Spark workspace server",
            "After=network.target",
            "",
            "[Service]",
            "Type=simple",
            "Restart=on-failure",
            "RestartSec=2",
            f"EnvironmentFile=-{provider_env_file}",
            *(f"Environment={entry}" for entry in service_environment),
            f"ExecStart={exec_start}",
            "",
            "[Install]",
            "WantedBy=default.target",
            "",
        ]
    )


def _quote_systemd_arg(value: str) -> str:
    escaped = value.replace("%", "%%").replace("\\", "\\\\").replace('"', '\\"')
    if not escaped or any(char.isspace() for char in escaped):
        return f'"{escaped}"'
    return escaped


def _run_systemctl(systemctl: str, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [systemctl, "--user", *args],
        check=check,
        capture_output=True,
        text=True,
    )


def _print_systemctl_failure(exc: subprocess.CalledProcessError) -> None:
    stderr = (exc.stderr or "").strip()
    stdout = (exc.stdout or "").strip()
    details = stderr or stdout or f"systemctl exited with status {exc.returncode}"
    print(details, file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
