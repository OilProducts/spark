from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Sequence
from urllib.parse import quote

import httpx

from workspace.project_chat_common import (
    normalize_flow_run_request_payload,
    normalize_spec_edit_proposal_payload,
)


DEFAULT_API_BASE_URL = "http://127.0.0.1:8000"
EXIT_GENERAL_FAILURE = 1
EXIT_USAGE_ERROR = 2
EXIT_NOT_FOUND = 3
SPEC_PROPOSAL_CREATE_EXAMPLE = """{
  "summary": "Clarify the approval gate before planning begins.",
  "changes": [
    {
      "path": "specs/sparkspawn-workspace.md#proposal-review",
      "before": "Planning begins immediately after a proposal is drafted.",
      "after": "Planning begins only after the user approves the proposal."
    }
  ],
  "rationale": "Ground the workflow in an explicit user approval step."
}"""
SPEC_PROPOSAL_CREATE_STDIN_EXAMPLE = """cat <<'EOF' | sparkspawn-workspace spec-proposal --json -
{
  "summary": "Clarify the approval gate before planning begins.",
  "changes": [
    {
      "path": "specs/sparkspawn-workspace.md#proposal-review",
      "before": "Planning begins immediately after a proposal is drafted.",
      "after": "Planning begins only after the user approves the proposal."
    }
  ],
  "rationale": "Ground the workflow in an explicit user approval step."
}
EOF"""
SPEC_PROPOSAL_CREATE_TEMPFILE_EXAMPLE = '''payload_file=$(mktemp)
cat >"$payload_file" <<'EOF'
{
  "summary": "Clarify the approval gate before planning begins.",
  "changes": [
    {
      "path": "specs/sparkspawn-workspace.md#proposal-review",
      "before": "Planning begins immediately after a proposal is drafted.",
      "after": "Planning begins only after the user approves the proposal."
    }
  ]
}
EOF
sparkspawn-workspace spec-proposal --conversation amber-otter --json "$payload_file"'''
FLOW_RUN_STDIN_EXAMPLE = """cat <<'EOF' | sparkspawn-workspace flow-run --conversation amber-otter --flow implement-spec.dot --summary "Run implementation for the approved scope" --goal -
Implement the approved work items from the current conversation state.
EOF"""
FLOW_RUN_TEMPFILE_EXAMPLE = '''goal_file=$(mktemp)
cat >"$goal_file" <<'EOF'
Implement the approved work items from the current conversation state.
EOF
sparkspawn-workspace flow-run --conversation amber-otter --flow implement-spec.dot --summary "Run implementation for the approved scope" --goal-file "$goal_file"'''
LIST_FLOWS_TEXT_EXAMPLE = "sparkspawn-workspace list-flows --text"
DESCRIBE_FLOW_TEXT_EXAMPLE = "sparkspawn-workspace describe-flow --flow implement-spec.dot --text"
GET_FLOW_TEXT_EXAMPLE = "sparkspawn-workspace get-flow --flow implement-spec.dot --text"


def _build_runtime_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="sparkspawn",
        description="Spark Spawn operator CLI",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    subparsers = parser.add_subparsers(dest="command")

    serve = subparsers.add_parser(
        "serve",
        help="Start the Sparkspawn API server",
    )
    serve.add_argument("--host", default="127.0.0.1", help="Bind host (default: 127.0.0.1)")
    serve.add_argument("--port", type=int, default=8000, help="Bind port (default: 8000)")
    serve.add_argument("--reload", action="store_true", help="Enable auto-reload (development only)")
    serve.add_argument("--data-dir", type=Path, default=None, help="Runtime data directory root")
    serve.add_argument("--flows-dir", type=Path, default=None, help="Flow storage directory")
    serve.add_argument("--ui-dir", type=Path, default=None, help="Built UI directory (contains index.html)")

    return parser


def _build_workspace_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="sparkspawn-workspace",
        description="Spark Spawn workspace agent CLI",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    subparsers = parser.add_subparsers(dest="command")

    spec_proposal = subparsers.add_parser(
        "spec-proposal",
        help="Create a pending spec proposal artifact in a conversation",
        description=(
            "Create a pending spec proposal artifact and inline conversation segment.\n\n"
            "The payload must include:\n"
            "  summary: short description of the proposed spec change\n"
            "  changes: minimal grounded before/after edits\n"
            "  rationale: optional explanation for the proposal\n\n"
            "The command requires a conversation handle supplied with --conversation.\n"
            "The command resolves the target assistant turn automatically.\n"
            "Do not include turn_id in the payload.\n"
            "This command does not approve or apply the proposal.\n\n"
            "Prefer piping the payload with --json - so no proposal file is left behind.\n"
            "If stdin is not practical, write the payload to a temporary file outside the project repository."
        ),
        epilog=(
            "Preferred stdin example:\n"
            + SPEC_PROPOSAL_CREATE_STDIN_EXAMPLE
            + "\n\nFallback temp-file example:\n"
            + SPEC_PROPOSAL_CREATE_TEMPFILE_EXAMPLE
            + "\n\nExample JSON payload:\n"
            + SPEC_PROPOSAL_CREATE_EXAMPLE
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    spec_proposal.add_argument(
        "--json",
        dest="json_path",
        required=True,
        help="Path to a JSON payload file, or '-' to read the payload from stdin.",
    )
    spec_proposal.add_argument(
        "--conversation",
        required=True,
        help="Conversation handle in adjective-noun form, for example 'amber-otter'.",
    )
    spec_proposal.add_argument(
        "--base-url",
        default=os.environ.get("SPARKSPAWN_API_BASE_URL", DEFAULT_API_BASE_URL),
        help=f"Sparkspawn server base URL (default: {DEFAULT_API_BASE_URL}).",
    )

    flow_run = subparsers.add_parser(
        "flow-run",
        help="Create a pending flow-run request artifact in a conversation",
        description=(
            "Create a pending flow-run request artifact and inline conversation segment.\n\n"
            "The command requires:\n"
            "  --flow: the Attractor flow name to request\n"
            "  --summary: short explanation for why the run should start\n"
            "  --conversation: the target conversation handle\n\n"
            "Use --goal-file or --goal - when the flow needs launch context.\n"
            "The command does not approve the request or start the run immediately."
        ),
        epilog=(
            "Preferred stdin example:\n"
            + FLOW_RUN_STDIN_EXAMPLE
            + "\n\nFallback temp-file example:\n"
            + FLOW_RUN_TEMPFILE_EXAMPLE
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    flow_run.add_argument(
        "--conversation",
        required=True,
        help="Conversation handle in adjective-noun form, for example 'amber-otter'.",
    )
    flow_run.add_argument(
        "--flow",
        required=True,
        help="Attractor flow name to request, for example 'implement-spec.dot'.",
    )
    flow_run.add_argument(
        "--summary",
        required=True,
        help="Short human-readable reason for the requested run.",
    )
    goal_group = flow_run.add_mutually_exclusive_group()
    goal_group.add_argument(
        "--goal",
        dest="goal_text",
        help="Inline goal text, or '-' to read the goal from stdin.",
    )
    goal_group.add_argument(
        "--goal-file",
        dest="goal_file",
        help="Path to a text file containing the optional run goal.",
    )
    flow_run.add_argument(
        "--model",
        help="Optional model override to request if the run is approved.",
    )
    flow_run.add_argument(
        "--base-url",
        default=os.environ.get("SPARKSPAWN_API_BASE_URL", DEFAULT_API_BASE_URL),
        help=f"Sparkspawn server base URL (default: {DEFAULT_API_BASE_URL}).",
    )

    list_flows = subparsers.add_parser(
        "list-flows",
        help="List agent-requestable workspace flows",
        description=(
            "List the flows that the workspace exposes for independent agent initiation.\n\n"
            "JSON is the default stdout format so agents can consume the output reliably.\n"
            "Use --text for human-readable output."
        ),
        epilog=f"Example:\n{LIST_FLOWS_TEXT_EXAMPLE}",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    list_flows.add_argument(
        "--text",
        action="store_true",
        help="Render human-readable text instead of the default JSON output.",
    )
    list_flows.add_argument(
        "--base-url",
        default=os.environ.get("SPARKSPAWN_API_BASE_URL", DEFAULT_API_BASE_URL),
        help=f"Sparkspawn server base URL (default: {DEFAULT_API_BASE_URL}).",
    )

    describe_flow = subparsers.add_parser(
        "describe-flow",
        help="Describe one agent-requestable workspace flow",
        description=(
            "Describe a single flow exposed for agent initiation.\n\n"
            "JSON is the default stdout format so agents can read the result without text parsing.\n"
            "Use --text for a human-readable view."
        ),
        epilog=f"Example:\n{DESCRIBE_FLOW_TEXT_EXAMPLE}",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    describe_flow.add_argument(
        "--flow",
        required=True,
        help="Flow file name, for example 'implement-spec.dot'.",
    )
    describe_flow.add_argument(
        "--text",
        action="store_true",
        help="Render human-readable text instead of the default JSON output.",
    )
    describe_flow.add_argument(
        "--base-url",
        default=os.environ.get("SPARKSPAWN_API_BASE_URL", DEFAULT_API_BASE_URL),
        help=f"Sparkspawn server base URL (default: {DEFAULT_API_BASE_URL}).",
    )

    get_flow = subparsers.add_parser(
        "get-flow",
        help="Fetch the raw DOT for one agent-requestable workspace flow",
        description=(
            "Fetch the raw DOT source for a single flow exposed for agent initiation.\n\n"
            "JSON is the default stdout format and wraps the content with the flow name.\n"
            "Use --text to print raw DOT directly."
        ),
        epilog=f"Example:\n{GET_FLOW_TEXT_EXAMPLE}",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    get_flow.add_argument(
        "--flow",
        required=True,
        help="Flow file name, for example 'implement-spec.dot'.",
    )
    get_flow.add_argument(
        "--text",
        action="store_true",
        help="Print raw DOT only instead of the default JSON wrapper.",
    )
    get_flow.add_argument(
        "--base-url",
        default=os.environ.get("SPARKSPAWN_API_BASE_URL", DEFAULT_API_BASE_URL),
        help=f"Sparkspawn server base URL (default: {DEFAULT_API_BASE_URL}).",
    )

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _build_runtime_parser()
    args = parser.parse_args(argv)

    if not getattr(args, "command", None):
        parser.print_help()
        return 0

    if args.command == "serve":
        return _run_serve(args)

    parser.error(f"Unknown command: {args.command}")
    return 2


def workspace_main(argv: Sequence[str] | None = None) -> int:
    parser = _build_workspace_parser()
    args = parser.parse_args(argv)

    if not getattr(args, "command", None):
        parser.print_help()
        return 0

    if args.command == "spec-proposal":
        return _run_spec_proposal(args)
    if args.command == "flow-run":
        return _run_flow_run(args)
    if args.command == "list-flows":
        return _run_list_flows(args)
    if args.command == "describe-flow":
        return _run_describe_flow(args)
    if args.command == "get-flow":
        return _run_get_flow(args)

    parser.error(f"Unknown command: {args.command}")
    return EXIT_USAGE_ERROR


def _run_spec_proposal(args: argparse.Namespace) -> int:
    try:
        raw_payload = _read_json_input(str(args.json_path))
        payload = json.loads(raw_payload)
    except FileNotFoundError:
        print(json.dumps({"ok": False, "error": f"JSON file not found: {args.json_path}"}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE
    except json.JSONDecodeError as exc:
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": (
                        f"Invalid JSON in {args.json_path} at line {exc.lineno}, column {exc.colno}: {exc.msg}. "
                        "Fix the JSON syntax and try again."
                    ),
                }
            ),
            file=sys.stderr,
        )
        return EXIT_GENERAL_FAILURE

    if not isinstance(payload, dict):
        print(json.dumps({"ok": False, "error": "Proposal payload must be a JSON object."}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    unexpected_keys = sorted(set(payload.keys()) - {"summary", "changes", "rationale"})
    if unexpected_keys:
        if "conversation_id" in unexpected_keys:
            error = (
                "Do not include conversation_id in the JSON payload. "
                "Pass the target thread with --conversation adjective-noun instead."
            )
        elif "turn_id" in unexpected_keys:
            error = (
                "Do not include turn_id in the JSON payload. "
                "Spark Spawn places the proposal on the correct assistant turn automatically."
            )
        elif "project_path" in unexpected_keys:
            error = (
                "Do not include project_path in the JSON payload. "
                "The conversation handle already determines the owning project."
            )
        else:
            keys = ", ".join(unexpected_keys)
            error = (
                f"Unexpected payload field(s): {keys}. "
                "Allowed fields are summary, changes, and optional rationale."
            )
        print(json.dumps({"ok": False, "error": error}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    conversation_handle = str(args.conversation or "").strip()
    if not conversation_handle:
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": "Missing required --conversation handle. Use the adjective-noun handle shown for the thread, for example 'amber-otter'.",
                }
            ),
            file=sys.stderr,
        )
        return EXIT_GENERAL_FAILURE

    try:
        payload = normalize_spec_edit_proposal_payload(
            payload,
            source_name="sparkspawn-workspace spec-proposal",
        )
    except ValueError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    base_url = str(args.base_url).rstrip("/")
    request_url = f"{base_url}/workspace/api/conversations/by-handle/{conversation_handle}/spec-edit-proposals"

    try:
        with httpx.Client(timeout=30.0) as client:
            response = client.post(request_url, json=payload)
    except httpx.HTTPError as exc:
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": f"Request failed: {exc}",
                }
            ),
            file=sys.stderr,
        )
        return EXIT_GENERAL_FAILURE

    try:
        response_payload = response.json()
    except ValueError:
        response_payload = {"detail": response.text}

    if response.is_error:
        error_detail: object
        if isinstance(response_payload, dict):
            error_detail = response_payload.get("detail")
        else:
            error_detail = response.text
        if isinstance(error_detail, list):
            message = "; ".join(_format_validation_error(entry) for entry in error_detail)
        elif isinstance(error_detail, str):
            message = error_detail
        else:
            message = response.text
        print(
            json.dumps(
                {
                    "ok": False,
                    "status_code": response.status_code,
                    "error": message,
                }
            ),
            file=sys.stderr,
        )
        return EXIT_GENERAL_FAILURE

    print(json.dumps(response_payload, indent=2, sort_keys=True))
    return 0


def _read_json_input(json_path: str) -> str:
    if json_path == "-":
        return sys.stdin.read()
    return Path(json_path).read_text(encoding="utf-8")


def _read_optional_goal(args: argparse.Namespace) -> str | None:
    goal_text = str(getattr(args, "goal_text", "") or "").strip()
    goal_file = str(getattr(args, "goal_file", "") or "").strip()
    if goal_text:
        if goal_text == "-":
            text = sys.stdin.read().strip()
            return text or None
        return goal_text
    if goal_file:
        return Path(goal_file).read_text(encoding="utf-8").strip() or None
    return None


def _run_flow_run(args: argparse.Namespace) -> int:
    conversation_handle = str(args.conversation or "").strip()
    if not conversation_handle:
        print(json.dumps({"ok": False, "error": "Missing required --conversation handle."}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    try:
        goal = _read_optional_goal(args)
    except FileNotFoundError:
        print(json.dumps({"ok": False, "error": f"Goal file not found: {args.goal_file}"}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    try:
        payload = normalize_flow_run_request_payload(
            {
                "flow_name": args.flow,
                "summary": args.summary,
                "goal": goal,
                "model": args.model,
            },
            source_name="sparkspawn-workspace flow-run",
        )
    except ValueError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    base_url = str(args.base_url).rstrip("/")
    request_url = f"{base_url}/workspace/api/conversations/by-handle/{conversation_handle}/flow-run-requests"

    try:
        with httpx.Client(timeout=30.0) as client:
            response = client.post(request_url, json=payload)
    except httpx.HTTPError as exc:
        print(json.dumps({"ok": False, "error": f"Request failed: {exc}"}), file=sys.stderr)
        return EXIT_GENERAL_FAILURE

    try:
        response_payload = response.json()
    except ValueError:
        response_payload = {"detail": response.text}

    if response.is_error:
        error_detail: object
        if isinstance(response_payload, dict):
            error_detail = response_payload.get("detail")
        else:
            error_detail = response.text
        if isinstance(error_detail, list):
            message = "; ".join(_format_validation_error(entry) for entry in error_detail)
        elif isinstance(error_detail, str):
            message = error_detail
        else:
            message = response.text
        print(
            json.dumps(
                {
                    "ok": False,
                    "status_code": response.status_code,
                    "error": message,
                }
            ),
            file=sys.stderr,
        )
        return EXIT_GENERAL_FAILURE

    print(json.dumps(response_payload, indent=2, sort_keys=True))
    return 0


def _run_list_flows(args: argparse.Namespace) -> int:
    response_payload, exit_code = _request_json("GET", _workspace_agent_url(args.base_url, "/workspace/api/flows?surface=agent"))
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
    flow_name = str(args.flow or "").strip()
    if not flow_name:
        _print_error_payload({"ok": False, "error": "Missing required --flow name."})
        return EXIT_GENERAL_FAILURE
    response_payload, exit_code = _request_json(
        "GET",
        _workspace_agent_url(args.base_url, f"/workspace/api/flows/{quote(flow_name, safe='')}?surface=agent"),
    )
    if response_payload is None:
        return exit_code
    if args.text:
        _print_describe_flow_text(response_payload)
        return 0
    _print_success_payload(response_payload)
    return 0


def _run_get_flow(args: argparse.Namespace) -> int:
    flow_name = str(args.flow or "").strip()
    if not flow_name:
        _print_error_payload({"ok": False, "error": "Missing required --flow name."})
        return EXIT_GENERAL_FAILURE
    response_text, exit_code = _request_text(
        "GET",
        _workspace_agent_url(args.base_url, f"/workspace/api/flows/{quote(flow_name, safe='')}/raw?surface=agent"),
    )
    if response_text is None:
        return exit_code
    if args.text:
        print(response_text, end="" if response_text.endswith("\n") else "\n")
        return 0
    _print_success_payload({"name": flow_name, "content": response_text})
    return 0


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


def _workspace_agent_url(base_url: str, path: str) -> str:
    return f"{str(base_url).rstrip('/')}{path}"


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
    print(f"Graph Goal: {graph_goal or '(none)'}")
    print(f"Node Count: {payload.get('node_count')}")
    print(f"Edge Count: {payload.get('edge_count')}")
    features = payload.get("features")
    if isinstance(features, dict):
        print(f"Has Human Gate: {bool(features.get('has_human_gate'))}")
        print(f"Has Manager Loop: {bool(features.get('has_manager_loop'))}")


def _run_serve(args: argparse.Namespace) -> int:
    import uvicorn

    import attractor.api.server as server
    from attractor.config import ENV_FLOWS_DIR, ENV_HOME_DIR, ENV_UI_DIR

    def _set_path_env(name: str, value: Path | None) -> None:
        if value is None:
            return
        os.environ[name] = str(value.expanduser().resolve(strict=False))

    server.configure_runtime_paths(
        data_dir=args.data_dir,
        flows_dir=args.flows_dir,
        ui_dir=args.ui_dir,
    )
    server.validate_runtime_paths()
    _set_path_env(ENV_HOME_DIR, args.data_dir)
    _set_path_env(ENV_FLOWS_DIR, args.flows_dir)
    _set_path_env(ENV_UI_DIR, args.ui_dir)

    uvicorn.run(
        "attractor.api.server:app" if args.reload else server.app,
        host=args.host,
        port=args.port,
        reload=args.reload,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
