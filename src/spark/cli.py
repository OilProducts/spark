from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
from typing import Any, Mapping, Sequence
from urllib.parse import quote

import httpx

from attractor.validation_preview import preview_dot_source
from spark.chat.response_parsing import (
    normalize_flow_run_request_payload,
)


DEFAULT_API_BASE_URL = "http://127.0.0.1:8000"
EXIT_GENERAL_FAILURE = 1
EXIT_USAGE_ERROR = 2
EXIT_NOT_FOUND = 3


def _build_agent_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="spark",
        description="Spark agent CLI",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    domains = parser.add_subparsers(dest="domain")

    convo = domains.add_parser("convo", help="Conversation-scoped artifact commands")
    convo_commands = convo.add_subparsers(dest="command")

    run_request = convo_commands.add_parser(
        "run-request",
        help="Create a pending run-request artifact in a conversation",
    )
    run_request.add_argument("--conversation", required=True, help="Conversation handle in adjective-noun form.")
    run_request.add_argument(
        "--flow",
        required=True,
        help="Flow name, for example 'software-development/spec-implementation/implement-spec.dot'.",
    )
    run_request.add_argument("--summary", required=True, help="Short explanation for the requested run.")
    goal_group = run_request.add_mutually_exclusive_group()
    goal_group.add_argument("--goal", dest="goal_text", help="Inline goal text, or '-' to read from stdin.")
    goal_group.add_argument("--goal-file", dest="goal_file", help="Path to a text file containing the optional goal.")
    launch_context_group = run_request.add_mutually_exclusive_group()
    launch_context_group.add_argument("--launch-context-json", dest="launch_context_json")
    launch_context_group.add_argument("--launch-context-file", dest="launch_context_file")
    run_request.add_argument("--model", help="Optional model override to request if approved.")
    run_request.add_argument("--llm-provider", dest="llm_provider", help="Optional LLM provider override to request if approved.")
    run_request.add_argument("--reasoning-effort", dest="reasoning_effort", help="Optional reasoning effort override to request if approved.")
    run_request.add_argument("--base-url")

    run = domains.add_parser("run", help="Direct execution commands")
    run_commands = run.add_subparsers(dest="command")

    launch = run_commands.add_parser(
        "launch",
        help="Launch a flow immediately",
    )
    launch.add_argument(
        "--flow",
        required=True,
        help="Flow name, for example 'software-development/spec-implementation/implement-spec.dot'.",
    )
    launch.add_argument("--summary", required=True, help="Short explanation for the launch.")
    launch.add_argument("--conversation", help="Conversation handle in adjective-noun form.")
    launch.add_argument("--project", dest="project_path", help="Explicit project path when not launching from conversation context.")
    goal_group = launch.add_mutually_exclusive_group()
    goal_group.add_argument("--goal", dest="goal_text", help="Inline goal text, or '-' to read from stdin.")
    goal_group.add_argument("--goal-file", dest="goal_file", help="Path to a text file containing the optional goal.")
    launch_context_group = launch.add_mutually_exclusive_group()
    launch_context_group.add_argument("--launch-context-json", dest="launch_context_json")
    launch_context_group.add_argument("--launch-context-file", dest="launch_context_file")
    launch.add_argument("--model", help="Optional model override.")
    launch.add_argument("--llm-provider", dest="llm_provider", help="Optional LLM provider override.")
    launch.add_argument("--reasoning-effort", dest="reasoning_effort", help="Optional reasoning effort override.")
    launch.add_argument("--base-url")

    flow = domains.add_parser("flow", help="Flow discovery and validation")
    flow_commands = flow.add_subparsers(dest="command")

    flow_list = flow_commands.add_parser("list", help="List agent-requestable workspace flows")
    flow_list.add_argument("--text", action="store_true", help="Render human-readable text instead of JSON.")
    flow_list.add_argument("--base-url")

    flow_describe = flow_commands.add_parser("describe", help="Describe one agent-requestable flow")
    flow_describe.add_argument("--flow", required=True)
    flow_describe.add_argument("--text", action="store_true")
    flow_describe.add_argument("--base-url")

    flow_get = flow_commands.add_parser("get", help="Fetch raw DOT for one agent-requestable flow")
    flow_get.add_argument("--flow", required=True)
    flow_get.add_argument("--text", action="store_true")
    flow_get.add_argument("--base-url")

    flow_validate = flow_commands.add_parser("validate", help="Validate a flow after direct DOT edits")
    flow_validate_target = flow_validate.add_mutually_exclusive_group(required=True)
    flow_validate_target.add_argument("--flow")
    flow_validate_target.add_argument("--file")
    flow_validate.add_argument("--text", action="store_true")
    flow_validate.add_argument("--base-url")

    trigger = domains.add_parser("trigger", help="Workspace trigger management")
    trigger_commands = trigger.add_subparsers(dest="command")

    trigger_list = trigger_commands.add_parser("list", help="List workspace triggers")
    trigger_list.add_argument("--text", action="store_true")
    trigger_list.add_argument("--base-url")

    trigger_describe = trigger_commands.add_parser("describe", help="Describe one workspace trigger")
    trigger_describe.add_argument("--id", required=True, help="Trigger id.")
    trigger_describe.add_argument("--text", action="store_true")
    trigger_describe.add_argument("--base-url")

    trigger_create = trigger_commands.add_parser("create", help="Create a workspace trigger from JSON")
    trigger_create.add_argument("--json", dest="json_path", required=True, help="JSON payload file path, or '-' for stdin.")
    trigger_create.add_argument("--base-url")

    trigger_update = trigger_commands.add_parser("update", help="Patch a workspace trigger from JSON")
    trigger_update.add_argument("--id", required=True, help="Trigger id.")
    trigger_update.add_argument("--json", dest="json_path", required=True, help="JSON payload file path, or '-' for stdin.")
    trigger_update.add_argument("--base-url")

    trigger_delete = trigger_commands.add_parser("delete", help="Delete a workspace trigger")
    trigger_delete.add_argument("--id", required=True, help="Trigger id.")
    trigger_delete.add_argument("--base-url")

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _build_agent_parser()
    args = parser.parse_args(argv)

    if not getattr(args, "domain", None):
        parser.print_help()
        return 0

    if args.domain == "convo":
        if args.command == "run-request":
            return _run_run_request(args)
    if args.domain == "run":
        if args.command == "launch":
            return _run_launch(args)
    if args.domain == "flow":
        if args.command == "list":
            return _run_list_flows(args)
        if args.command == "describe":
            return _run_describe_flow(args)
        if args.command == "get":
            return _run_get_flow(args)
        if args.command == "validate":
            return _run_validate_flow(args)
    if args.domain == "trigger":
        if args.command == "list":
            return _run_list_triggers(args)
        if args.command == "describe":
            return _run_describe_trigger(args)
        if args.command == "create":
            return _run_create_trigger(args)
        if args.command == "update":
            return _run_update_trigger(args)
        if args.command == "delete":
            return _run_delete_trigger(args)

    parser.error("Unknown command")
    return EXIT_USAGE_ERROR


def _run_run_request(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark convo run-request")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    normalized = _build_flow_payload(
        flow_name=args.flow,
        summary=args.summary,
        goal_text=getattr(args, "goal_text", None),
        goal_file=getattr(args, "goal_file", None),
        launch_context_json=getattr(args, "launch_context_json", None),
        launch_context_file=getattr(args, "launch_context_file", None),
        model=getattr(args, "model", None),
        llm_provider=getattr(args, "llm_provider", None),
        reasoning_effort=getattr(args, "reasoning_effort", None),
        source_name="spark convo run-request",
    )
    if isinstance(normalized, tuple):
        _print_error_payload({"ok": False, "error": normalized[0]})
        return normalized[1]

    request_url = _workspace_url(
        base_url,
        f"/workspace/api/conversations/by-handle/{quote(str(args.conversation).strip(), safe='')}/flow-run-requests",
    )
    response_payload, exit_code = _request_json("POST", request_url, payload=normalized)
    if response_payload is None:
        return exit_code
    _print_success_payload(response_payload)
    return 0


def _run_launch(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark run launch")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    normalized = _build_flow_payload(
        flow_name=args.flow,
        summary=args.summary,
        goal_text=getattr(args, "goal_text", None),
        goal_file=getattr(args, "goal_file", None),
        launch_context_json=getattr(args, "launch_context_json", None),
        launch_context_file=getattr(args, "launch_context_file", None),
        model=getattr(args, "model", None),
        llm_provider=getattr(args, "llm_provider", None),
        reasoning_effort=getattr(args, "reasoning_effort", None),
        source_name="spark run launch",
    )
    if isinstance(normalized, tuple):
        _print_error_payload({"ok": False, "error": normalized[0]})
        return normalized[1]

    conversation = str(args.conversation or "").strip()
    project_path = str(args.project_path or "").strip()
    if not conversation and not project_path:
        _print_error_payload(
            {
                "ok": False,
                "error": "spark run launch requires --project when --conversation is omitted.",
            }
        )
        return EXIT_GENERAL_FAILURE

    payload = dict(normalized)
    if conversation:
        payload["conversation_handle"] = conversation
    if project_path:
        payload["project_path"] = project_path

    request_url = _workspace_url(base_url, "/workspace/api/runs/launch")
    response_payload, exit_code = _request_json("POST", request_url, payload=payload)
    if response_payload is None:
        return exit_code
    _print_success_payload(response_payload)
    return 0


def _run_list_flows(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark flow list")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json("GET", _workspace_url(base_url, "/workspace/api/flows?surface=agent"))
    if response_payload is None:
        return exit_code
    if args.text:
        rows = response_payload if isinstance(response_payload, list) else []
        for row in rows:
            if not isinstance(row, dict):
                continue
            name = str(row.get("name") or "").strip()
            title = str(row.get("title") or name).strip()
            description = str(row.get("description") or "").strip()
            line = f"{name}: {title}" if title and title != name else name
            print(line)
            if description:
                print(f"  {description}")
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_describe_flow(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark flow describe")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    flow_name = str(args.flow or "").strip()
    if not flow_name:
        _print_error_payload({"ok": False, "error": "Missing required --flow name."})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "GET",
        _workspace_url(base_url, f"/workspace/api/flows/{quote(flow_name, safe='')}?surface=agent"),
    )
    if response_payload is None:
        return exit_code
    if args.text:
        _print_describe_flow_text(response_payload)
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_get_flow(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark flow get")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    flow_name = str(args.flow or "").strip()
    if not flow_name:
        _print_error_payload({"ok": False, "error": "Missing required --flow name."})
        return EXIT_GENERAL_FAILURE
    response_text, exit_code = _request_text(
        "GET",
        _workspace_url(base_url, f"/workspace/api/flows/{quote(flow_name, safe='')}/raw?surface=agent"),
    )
    if response_text is None:
        return exit_code
    if args.text:
        print(response_text, end="" if response_text.endswith("\n") else "\n")
        return 0
    _print_success_payload({"name": flow_name, "content": response_text})
    return 0


def _run_validate_flow(args: argparse.Namespace) -> int:
    flow_file = str(getattr(args, "file", "") or "").strip()
    if flow_file:
        try:
            flow_path = Path(flow_file).expanduser()
            raw_content = flow_path.read_text(encoding="utf-8")
        except FileNotFoundError:
            _print_error_payload({"ok": False, "error": f"Flow file not found: {flow_file}"})
            return EXIT_GENERAL_FAILURE
        except OSError as exc:
            _print_error_payload({"ok": False, "error": f"Unable to read flow file {flow_file}: {exc}"})
            return EXIT_GENERAL_FAILURE

        _graph, preview_payload = preview_dot_source(raw_content)
        response_payload = {
            "name": flow_path.name,
            "path": str(flow_path.resolve(strict=False)),
            **preview_payload,
        }
    else:
        base_url = _resolve_base_url_or_print_error(args.base_url, command="spark flow validate")
        if base_url is None:
            return EXIT_GENERAL_FAILURE
        flow_name = str(args.flow or "").strip()
        if not flow_name:
            _print_error_payload({"ok": False, "error": "Missing required --flow name."})
            return EXIT_GENERAL_FAILURE
        response_payload, exit_code = _request_json(
            "GET",
            _workspace_url(base_url, f"/workspace/api/flows/{quote(flow_name, safe='')}/validate"),
        )
        if response_payload is None:
            return exit_code
    if args.text:
        _print_validate_flow_text(response_payload)
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_list_triggers(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark trigger list")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json("GET", _workspace_url(base_url, "/workspace/api/triggers"))
    if response_payload is None:
        return exit_code
    if args.text:
        rows = response_payload if isinstance(response_payload, list) else []
        for row in rows:
            if not isinstance(row, dict):
                continue
            name = str(row.get("name") or "").strip()
            trigger_id = str(row.get("id") or "").strip()
            source_type = str(row.get("source_type") or "").strip()
            enabled = bool(row.get("enabled"))
            protected = bool(row.get("protected"))
            flow_name = str(((row.get("action") or {}) if isinstance(row, dict) else {}).get("flow_name") or "").strip()
            print(f"{trigger_id}: {name} [{source_type}] -> {flow_name}")
            print(f"  enabled={enabled} protected={protected}")
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_describe_trigger(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark trigger describe")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    trigger_id = str(args.id or "").strip()
    if not trigger_id:
        _print_error_payload({"ok": False, "error": "Missing required --id."})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "GET",
        _workspace_url(base_url, f"/workspace/api/triggers/{quote(trigger_id, safe='')}"),
    )
    if response_payload is None:
        return exit_code
    if args.text:
        _print_describe_trigger_text(response_payload)
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_create_trigger(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark trigger create")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    try:
        payload = _read_required_json_object(str(args.json_path), "Trigger payload")
    except ValueError as exc:
        _print_error_payload({"ok": False, "error": str(exc)})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "POST",
        _workspace_url(base_url, "/workspace/api/triggers"),
        payload=payload,
    )
    if response_payload is None:
        return exit_code
    _print_success_payload(response_payload)
    return 0


def _run_update_trigger(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark trigger update")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    trigger_id = str(args.id or "").strip()
    if not trigger_id:
        _print_error_payload({"ok": False, "error": "Missing required --id."})
        return EXIT_GENERAL_FAILURE
    try:
        payload = _read_required_json_object(str(args.json_path), "Trigger payload")
    except ValueError as exc:
        _print_error_payload({"ok": False, "error": str(exc)})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "PATCH",
        _workspace_url(base_url, f"/workspace/api/triggers/{quote(trigger_id, safe='')}"),
        payload=payload,
    )
    if response_payload is None:
        return exit_code
    _print_success_payload(response_payload)
    return 0


def _run_delete_trigger(args: argparse.Namespace) -> int:
    base_url = _resolve_base_url_or_print_error(args.base_url, command="spark trigger delete")
    if base_url is None:
        return EXIT_GENERAL_FAILURE
    trigger_id = str(args.id or "").strip()
    if not trigger_id:
        _print_error_payload({"ok": False, "error": "Missing required --id."})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "DELETE",
        _workspace_url(base_url, f"/workspace/api/triggers/{quote(trigger_id, safe='')}"),
    )
    if response_payload is None:
        return exit_code
    _print_success_payload(response_payload)
    return 0


def _read_required_json_object(path_or_stdin: str, label: str) -> dict[str, Any]:
    try:
        raw_payload = _read_json_input(path_or_stdin)
    except FileNotFoundError:
        raise ValueError(f"{label} file not found: {path_or_stdin}") from None
    try:
        parsed = json.loads(raw_payload)
    except json.JSONDecodeError as exc:
        raise ValueError(
            f"Invalid JSON in {path_or_stdin} at line {exc.lineno}, column {exc.colno}: {exc.msg}."
        ) from exc
    if not isinstance(parsed, dict):
        raise ValueError(f"{label} must be a JSON object.")
    return parsed


def _read_json_input(json_path: str) -> str:
    if json_path == "-":
        return sys.stdin.read()
    return Path(json_path).read_text(encoding="utf-8")


def _read_optional_goal(goal_text: str | None, goal_file: str | None) -> str | None:
    normalized_goal_text = str(goal_text or "").strip()
    normalized_goal_file = str(goal_file or "").strip()
    if normalized_goal_text:
        if normalized_goal_text == "-":
            text = sys.stdin.read().strip()
            return text or None
        return normalized_goal_text
    if normalized_goal_file:
        return Path(normalized_goal_file).read_text(encoding="utf-8").strip() or None
    return None


def _read_optional_launch_context(launch_context_json: str | None, launch_context_file: str | None) -> dict[str, object] | None:
    normalized_json = str(launch_context_json or "").strip()
    normalized_file = str(launch_context_file or "").strip()
    if normalized_json:
        parsed = json.loads(normalized_json)
    elif normalized_file:
        parsed = json.loads(Path(normalized_file).read_text(encoding="utf-8"))
    else:
        return None
    if not isinstance(parsed, dict):
        raise ValueError("Launch context must be a JSON object.")
    return parsed


def _build_flow_payload(
    *,
    flow_name: str,
    summary: str,
    goal_text: str | None,
    goal_file: str | None,
    launch_context_json: str | None,
    launch_context_file: str | None,
    model: str | None,
    llm_provider: str | None,
    reasoning_effort: str | None,
    source_name: str,
) -> dict[str, Any] | tuple[str, int]:
    try:
        goal = _read_optional_goal(goal_text, goal_file)
    except FileNotFoundError:
        return (f"Goal file not found: {goal_file}", EXIT_GENERAL_FAILURE)
    try:
        launch_context = _read_optional_launch_context(launch_context_json, launch_context_file)
    except FileNotFoundError:
        return (f"Launch context file not found: {launch_context_file}", EXIT_GENERAL_FAILURE)
    except json.JSONDecodeError as exc:
        return (f"Launch context must be valid JSON: {exc}", EXIT_GENERAL_FAILURE)
    except ValueError as exc:
        return (str(exc), EXIT_GENERAL_FAILURE)

    try:
        return normalize_flow_run_request_payload(
            {
                "flow_name": flow_name,
                "summary": summary,
                "goal": goal,
                "launch_context": launch_context,
                "model": model,
                "llm_provider": llm_provider,
                "reasoning_effort": reasoning_effort,
            },
            source_name=source_name,
        )
    except ValueError as exc:
        return (str(exc), EXIT_GENERAL_FAILURE)


def _workspace_url(base_url: str, path: str) -> str:
    return f"{str(base_url).rstrip('/')}{path}"


def _running_from_source_checkout(project_root: Path) -> bool:
    return (
        (project_root / ".git").exists()
        or (
            (project_root / "pyproject.toml").is_file()
            and (project_root / "src" / "spark" / "flows").is_dir()
            and (project_root / "frontend").is_dir()
        )
    )


def _require_explicit_agent_base_url(
    *,
    command: str,
    base_url: str | None,
    env: Mapping[str, str] | None = None,
) -> None:
    env_map = env if env is not None else os.environ
    project_root = Path(__file__).resolve().parents[2]
    if not _running_from_source_checkout(project_root):
        return
    if str(base_url or "").strip() or str(env_map.get("SPARK_API_BASE_URL") or "").strip():
        return
    raise RuntimeError(
        "\n".join(
            [
                f"Refusing to use default API target {DEFAULT_API_BASE_URL} from a source checkout at {project_root}.",
                "",
                "The default API target is reserved for the installed or stable Spark instance.",
                f"Run the source checkout with an explicit dev server target before `{command}`, for example:",
                "",
                "  SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow list",
                "  SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow describe --flow examples/simple-linear.dot",
                "  uv run spark flow validate --file src/spark/flows/examples/simple-linear.dot --text",
            ]
        )
    )


def _resolve_base_url_or_print_error(base_url: str | None, *, command: str) -> str | None:
    try:
        _require_explicit_agent_base_url(command=command, base_url=base_url)
    except RuntimeError as exc:
        _print_error_payload({"ok": False, "error": str(exc)})
        return None
    return str(base_url or os.environ.get("SPARK_API_BASE_URL") or DEFAULT_API_BASE_URL).strip()


def _request_json(method: str, url: str, *, payload: dict[str, object] | None = None) -> tuple[object | None, int]:
    try:
        with httpx.Client(timeout=30.0) as client:
            response = client.request(method, url, json=payload)
    except httpx.HTTPError as exc:
        _print_error_payload({"ok": False, "error": f"Request failed: {exc}"})
        return None, EXIT_GENERAL_FAILURE
    return _handle_response_payload(response)


def _request_text(method: str, url: str) -> tuple[str | None, int]:
    try:
        with httpx.Client(timeout=30.0) as client:
            response = client.request(method, url)
    except httpx.HTTPError as exc:
        _print_error_payload({"ok": False, "error": f"Request failed: {exc}"})
        return None, EXIT_GENERAL_FAILURE
    if response.is_error:
        _print_error_payload(_build_error_payload(response))
        return None, _response_error_exit_code(response.status_code)
    return response.text, 0


def _handle_response_payload(response: httpx.Response) -> tuple[object | None, int]:
    try:
        response_payload: object = response.json()
    except ValueError:
        response_payload = {"detail": response.text}

    if response.is_error:
        _print_error_payload(_build_error_payload(response, response_payload))
        return None, _response_error_exit_code(response.status_code)
    return response_payload, 0


def _build_error_payload(response: httpx.Response, response_payload: object | None = None) -> dict[str, object]:
    payload = response_payload
    if payload is None:
        try:
            payload = response.json()
        except ValueError:
            payload = {"detail": response.text}
    error_detail: object
    if isinstance(payload, dict):
        error_detail = payload.get("detail")
    else:
        error_detail = response.text
    if isinstance(error_detail, list):
        message = "; ".join(_format_validation_error(entry) for entry in error_detail)
    elif isinstance(error_detail, str):
        message = error_detail
    else:
        message = response.text
    return {
        "ok": False,
        "status_code": response.status_code,
        "error": message,
    }


def _format_validation_error(value: object) -> str:
    if not isinstance(value, dict):
        return str(value)
    location = value.get("loc")
    message = str(value.get("msg") or "Invalid request.")
    if isinstance(location, (list, tuple)) and location:
        path = ".".join(str(part) for part in location if part != "body")
        if path:
            return f"{path}: {message}"
    return message


def _print_success_payload(payload: object) -> None:
    print(json.dumps(payload, indent=2, sort_keys=True))


def _print_error_payload(payload: dict[str, object]) -> None:
    print(json.dumps(payload), file=sys.stderr)


def _response_error_exit_code(status_code: int) -> int:
    return EXIT_NOT_FOUND if status_code == 404 else EXIT_GENERAL_FAILURE


def _print_describe_flow_text(payload: object) -> None:
    if not isinstance(payload, dict):
        print(str(payload))
        return
    name = str(payload.get("name") or "").strip()
    title = str(payload.get("title") or name).strip()
    description = str(payload.get("description") or "").strip()
    graph_label = str(payload.get("graph_label") or "").strip()
    graph_goal = str(payload.get("graph_goal") or "").strip()
    print(f"Name: {name}")
    print(f"Title: {title}")
    print(f"Description: {description or '(none)'}")
    print(f"Launch Policy: {payload.get('effective_launch_policy') or 'disabled'}")
    print(f"Graph Label: {graph_label or '(none)'}")
    print(f"Stated Goal: {graph_goal or '(none)'}")
    print(f"Node Count: {payload.get('node_count')}")
    print(f"Edge Count: {payload.get('edge_count')}")
    features = payload.get("features")
    if isinstance(features, dict):
        print(f"Has Human Gate: {bool(features.get('has_human_gate'))}")
        print(f"Has Manager Loop: {bool(features.get('has_manager_loop'))}")


def _print_validate_flow_text(payload: object) -> None:
    if not isinstance(payload, dict):
        print(str(payload))
        return
    name = str(payload.get("name") or "").strip()
    path = str(payload.get("path") or "").strip()
    status = str(payload.get("status") or "").strip()
    diagnostics = payload.get("diagnostics")
    errors = payload.get("errors")
    diagnostics_list = diagnostics if isinstance(diagnostics, list) else []
    errors_list = errors if isinstance(errors, list) else []
    print(f"Name: {name}")
    print(f"Path: {path or '(unknown)'}")
    print(f"Status: {status or '(unknown)'}")
    print(f"Diagnostics: {len(diagnostics_list)}")
    print(f"Errors: {len(errors_list)}")
    for diagnostic in diagnostics_list:
        if not isinstance(diagnostic, dict):
            continue
        severity = str(diagnostic.get("severity") or "info").strip().upper()
        rule = str(diagnostic.get("rule_id") or diagnostic.get("rule") or "").strip()
        message = str(diagnostic.get("message") or "").strip()
        line_value = diagnostic.get("line")
        line_suffix = f" line {line_value}" if isinstance(line_value, int) and line_value > 0 else ""
        rule_prefix = f" {rule}" if rule else ""
        print(f"- {severity}{rule_prefix}{line_suffix}: {message}")


def _print_describe_trigger_text(payload: object) -> None:
    if not isinstance(payload, dict):
        print(str(payload))
        return
    action = payload.get("action") if isinstance(payload.get("action"), dict) else {}
    state = payload.get("state") if isinstance(payload.get("state"), dict) else {}
    print(f"ID: {payload.get('id')}")
    print(f"Name: {payload.get('name')}")
    print(f"Source Type: {payload.get('source_type')}")
    print(f"Enabled: {bool(payload.get('enabled'))}")
    print(f"Protected: {bool(payload.get('protected'))}")
    print(f"Flow Target: {action.get('flow_name')}")
    print(f"Project Target: {action.get('project_path') or '(none)'}")
    print(f"Last Fired: {state.get('last_fired_at') or '(never)'}")
    print(f"Last Result: {state.get('last_result') or '(none)'}")
    print(f"Next Run: {state.get('next_run_at') or '(n/a)'}")
    webhook_secret = payload.get("webhook_secret")
    if webhook_secret:
        print(f"Webhook Secret: {webhook_secret}")


if __name__ == "__main__":
    raise SystemExit(main())
