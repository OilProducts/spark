from __future__ import annotations

import asyncio
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
from datetime import UTC, datetime, timedelta
import hashlib
import hmac
import json
import secrets
from pathlib import Path
from typing import Any, Optional, Protocol
import tomllib
import uuid

import httpx

from spark_common.logging import get_spark_logger
from spark_common.runtime import normalize_project_path
from workspace.attractor_client import AttractorApiClient, AttractorApiError


LOGGER = get_spark_logger("workspace.triggers")

TRIGGER_SOURCE_TYPES = {"schedule", "poll", "webhook", "workspace_event", "flow_event"}
WORKSPACE_TRIGGER_EVENTS = {
    "spec_edit_approved",
    "execution_card_approved",
    "execution_card_rejected",
    "execution_card_revision_requested",
}
TERMINAL_PIPELINE_STATUSES = {
    "completed",
    "failed",
    "validation_error",
    "canceled",
    "cancelled",
}
WEEKDAY_ORDER = ("mon", "tue", "wed", "thu", "fri", "sat", "sun")
MAX_RECENT_HISTORY = 20
MAX_DEDUPE_KEYS = 200
MAX_POLL_ITEM_IDS = 500
WEBHOOK_KEY_BYTES = 12
WEBHOOK_SECRET_BYTES = 24


class TriggerError(ValueError):
    pass


class TriggerSettings(Protocol):
    data_dir: Path
    config_dir: Path


@dataclass
class TriggerAction:
    flow_name: str
    project_path: str | None = None
    static_context: dict[str, Any] = field(default_factory=dict)


@dataclass
class TriggerDefinition:
    id: str
    name: str
    enabled: bool
    protected: bool
    source_type: str
    action: TriggerAction
    source: dict[str, Any]
    created_at: str
    updated_at: str


@dataclass
class TriggerState:
    last_fired_at: str | None = None
    last_result: str | None = None
    last_error: str | None = None
    next_run_at: str | None = None
    recent_history: list[dict[str, Any]] = field(default_factory=list)
    dedupe_keys: list[str] = field(default_factory=list)
    seen_item_ids: list[str] = field(default_factory=list)


def trigger_config_dir(config_dir: Path) -> Path:
    path = config_dir / "triggers"
    path.mkdir(parents=True, exist_ok=True)
    return path


def trigger_state_dir(data_dir: Path) -> Path:
    path = data_dir / "workspace" / "trigger-state"
    path.mkdir(parents=True, exist_ok=True)
    return path


def list_trigger_definitions(config_dir: Path) -> list[TriggerDefinition]:
    root = trigger_config_dir(config_dir)
    definitions: list[TriggerDefinition] = []
    for path in sorted(root.glob("*.toml")):
        try:
            definition = read_trigger_definition(config_dir, path.stem)
        except Exception as exc:  # noqa: BLE001
            LOGGER.warning("Failed to read trigger definition from %s: %s", path, exc)
            continue
        if definition is not None:
            definitions.append(definition)
    return definitions


def read_trigger_definition(config_dir: Path, trigger_id: str) -> TriggerDefinition | None:
    path = trigger_config_dir(config_dir) / f"{trigger_id}.toml"
    if not path.exists():
        return None
    payload = tomllib.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        return None
    return _parse_trigger_definition(trigger_id, payload)


def write_trigger_definition(config_dir: Path, definition: TriggerDefinition) -> None:
    path = trigger_config_dir(config_dir) / f"{definition.id}.toml"
    lines = [
        f'id = {_toml_string(definition.id)}',
        f'name = {_toml_string(definition.name)}',
        f'enabled = {_toml_bool(definition.enabled)}',
        f'protected = {_toml_bool(definition.protected)}',
        f'source_type = {_toml_string(definition.source_type)}',
        f'created_at = {_toml_string(definition.created_at)}',
        f'updated_at = {_toml_string(definition.updated_at)}',
        "",
        "[action]",
        f'flow_name = {_toml_string(definition.action.flow_name)}',
    ]
    if definition.action.project_path:
        lines.append(f'project_path = {_toml_string(definition.action.project_path)}')
    if definition.action.static_context:
        lines.append(f'static_context_json = {_toml_string(json.dumps(definition.action.static_context, sort_keys=True))}')
    lines.extend(
        [
            "",
            "[source]",
        ]
    )
    for key in sorted(definition.source.keys()):
        value = definition.source[key]
        if value is None:
            continue
        lines.extend(_toml_source_line(key, value))
    lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def delete_trigger_definition(config_dir: Path, trigger_id: str) -> None:
    path = trigger_config_dir(config_dir) / f"{trigger_id}.toml"
    if path.exists():
        path.unlink()


def delete_trigger_state(data_dir: Path, trigger_id: str) -> None:
    path = trigger_state_dir(data_dir) / f"{trigger_id}.json"
    if path.exists():
        path.unlink()


def load_trigger_state(data_dir: Path, trigger_id: str) -> TriggerState:
    path = trigger_state_dir(data_dir) / f"{trigger_id}.json"
    if not path.exists():
        return TriggerState()
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        LOGGER.warning("Failed to read trigger state from %s: %s", path, exc)
        return TriggerState()
    if not isinstance(payload, dict):
        return TriggerState()
    recent_history = payload.get("recent_history")
    dedupe_keys = payload.get("dedupe_keys")
    seen_item_ids = payload.get("seen_item_ids")
    return TriggerState(
        last_fired_at=_normalize_optional_string(payload.get("last_fired_at")),
        last_result=_normalize_optional_string(payload.get("last_result")),
        last_error=_normalize_optional_string(payload.get("last_error")),
        next_run_at=_normalize_optional_string(payload.get("next_run_at")),
        recent_history=[entry for entry in recent_history if isinstance(entry, dict)] if isinstance(recent_history, list) else [],
        dedupe_keys=[str(entry) for entry in dedupe_keys if isinstance(entry, str)] if isinstance(dedupe_keys, list) else [],
        seen_item_ids=[str(entry) for entry in seen_item_ids if isinstance(entry, str)] if isinstance(seen_item_ids, list) else [],
    )


def save_trigger_state(data_dir: Path, trigger_id: str, state: TriggerState) -> None:
    path = trigger_state_dir(data_dir) / f"{trigger_id}.json"
    path.write_text(json.dumps(asdict(state), indent=2, sort_keys=True) + "\n", encoding="utf-8")


def serialize_trigger(
    definition: TriggerDefinition,
    state: TriggerState,
    *,
    webhook_secret: str | None = None,
) -> dict[str, Any]:
    source_payload = dict(definition.source)
    if definition.source_type == "webhook":
        source_payload.pop("secret_hash", None)
    payload: dict[str, Any] = {
        "id": definition.id,
        "name": definition.name,
        "enabled": definition.enabled,
        "protected": definition.protected,
        "source_type": definition.source_type,
        "created_at": definition.created_at,
        "updated_at": definition.updated_at,
        "action": {
            "flow_name": definition.action.flow_name,
            "project_path": definition.action.project_path,
            "static_context": dict(definition.action.static_context),
        },
        "source": source_payload,
        "state": asdict(state),
    }
    if webhook_secret is not None:
        payload["webhook_secret"] = webhook_secret
    return payload


def validate_trigger_definition_payload(
    *,
    name: str,
    enabled: bool,
    source_type: str,
    action: Mapping[str, Any],
    source: Mapping[str, Any],
    protected: bool = False,
    trigger_id: str | None = None,
    created_at: str | None = None,
    updated_at: str | None = None,
) -> TriggerDefinition:
    normalized_id = trigger_id or f"trigger-{uuid.uuid4().hex[:12]}"
    normalized_name = name.strip()
    if not normalized_name:
        raise TriggerError("Trigger name is required.")
    normalized_source_type = source_type.strip()
    if normalized_source_type not in TRIGGER_SOURCE_TYPES:
        raise TriggerError(f"Unsupported trigger source type: {source_type}")
    normalized_action = _normalize_action(action)
    normalized_source = _normalize_source(normalized_source_type, source)
    now = _iso_now()
    return TriggerDefinition(
        id=normalized_id,
        name=normalized_name,
        enabled=bool(enabled),
        protected=bool(protected),
        source_type=normalized_source_type,
        action=normalized_action,
        source=normalized_source,
        created_at=created_at or now,
        updated_at=updated_at or now,
    )


def create_trigger_definition(
    config_dir: Path,
    *,
    name: str,
    enabled: bool,
    source_type: str,
    action: Mapping[str, Any],
    source: Mapping[str, Any],
    protected: bool = False,
) -> tuple[TriggerDefinition, str | None]:
    normalized_source = dict(source)
    webhook_secret: str | None = None
    if source_type == "webhook":
        webhook_key, webhook_secret, secret_hash = generate_webhook_credentials()
        normalized_source = {
            **normalized_source,
            "webhook_key": webhook_key,
            "secret_hash": secret_hash,
        }
    definition = validate_trigger_definition_payload(
        name=name,
        enabled=enabled,
        source_type=source_type,
        action=action,
        source=normalized_source,
        protected=protected,
    )
    write_trigger_definition(config_dir, definition)
    return definition, webhook_secret


def update_trigger_definition(
    config_dir: Path,
    trigger_id: str,
    *,
    name: str | None = None,
    enabled: bool | None = None,
    action: Mapping[str, Any] | None = None,
    source: Mapping[str, Any] | None = None,
    regenerate_webhook_secret: bool = False,
) -> tuple[TriggerDefinition, str | None]:
    existing = read_trigger_definition(config_dir, trigger_id)
    if existing is None:
        raise TriggerError("Unknown trigger.")
    if existing.protected:
        if source is not None:
            raise TriggerError("Protected triggers do not allow source changes.")
        if action is not None:
            next_action = _normalize_action({**asdict(existing.action), **dict(action)})
            if next_action.project_path != existing.action.project_path:
                raise TriggerError("Protected triggers do not allow project target changes.")
            if next_action.static_context != existing.action.static_context:
                raise TriggerError("Protected triggers do not allow static context changes.")
        if regenerate_webhook_secret:
            raise TriggerError("Protected triggers do not support webhook secret regeneration.")
    next_source = dict(existing.source)
    webhook_secret: str | None = None
    if source is not None:
        next_source = _normalize_source(existing.source_type, source, preserve_secret_hash=existing.source.get("secret_hash"))
    if regenerate_webhook_secret:
        if existing.source_type != "webhook":
            raise TriggerError("Only webhook triggers can regenerate webhook secrets.")
        webhook_key = str(next_source.get("webhook_key") or existing.source.get("webhook_key") or "")
        if not webhook_key:
            raise TriggerError("Webhook trigger is missing a routing key.")
        _, webhook_secret, secret_hash = generate_webhook_credentials(webhook_key=webhook_key)
        next_source["secret_hash"] = secret_hash
    next_action = existing.action if action is None else _normalize_action({**asdict(existing.action), **dict(action)})
    definition = validate_trigger_definition_payload(
        name=name if name is not None else existing.name,
        enabled=enabled if enabled is not None else existing.enabled,
        source_type=existing.source_type,
        action=asdict(next_action),
        source=next_source,
        protected=existing.protected,
        trigger_id=existing.id,
        created_at=existing.created_at,
        updated_at=_iso_now(),
    )
    write_trigger_definition(config_dir, definition)
    return definition, webhook_secret


def verify_webhook_secret(definition: TriggerDefinition, provided_secret: str) -> bool:
    secret_hash = str(definition.source.get("secret_hash") or "").strip()
    if not secret_hash or not provided_secret:
        return False
    expected_hash = hashlib.sha256(provided_secret.encode("utf-8")).hexdigest()
    return hmac.compare_digest(secret_hash, expected_hash)


def generate_webhook_credentials(*, webhook_key: str | None = None) -> tuple[str, str, str]:
    key = webhook_key or secrets.token_urlsafe(WEBHOOK_KEY_BYTES)
    secret = secrets.token_urlsafe(WEBHOOK_SECRET_BYTES)
    secret_hash = hashlib.sha256(secret.encode("utf-8")).hexdigest()
    return key, secret, secret_hash


def get_trigger_by_webhook_key(config_dir: Path, webhook_key: str) -> TriggerDefinition | None:
    for definition in list_trigger_definitions(config_dir):
        if definition.source_type != "webhook":
            continue
        if str(definition.source.get("webhook_key") or "").strip() == webhook_key.strip():
            return definition
    return None


def compute_next_run_at(definition: TriggerDefinition, state: TriggerState, *, now: datetime | None = None) -> str | None:
    if definition.source_type == "schedule":
        return _compute_schedule_next_run_at(definition.source, state, now=now)
    if definition.source_type == "poll":
        next_run_at = _normalize_optional_string(state.next_run_at)
        if next_run_at:
            return next_run_at
        interval_seconds = int(definition.source["interval_seconds"])
        return _datetime_to_iso((now or _utc_now()) + timedelta(seconds=interval_seconds))
    return None


class TriggerRuntime:
    def __init__(
        self,
        *,
        get_settings: callable,
        get_attractor_client: callable,
    ) -> None:
        self._get_settings = get_settings
        self._get_attractor_client = get_attractor_client
        self._definitions: dict[str, TriggerDefinition] = {}
        self._states: dict[str, TriggerState] = {}
        self._running_trigger_ids: set[str] = set()
        self._monitored_run_ids: set[str] = set()
        self._loop_task: asyncio.Task[None] | None = None
        self._lock = asyncio.Lock()

    async def start(self) -> None:
        async with self._lock:
            await self.reload()
            if self._loop_task is None or self._loop_task.done():
                self._loop_task = asyncio.create_task(self._run_loop())

    async def stop(self) -> None:
        async with self._lock:
            if self._loop_task is not None:
                self._loop_task.cancel()
                try:
                    await self._loop_task
                except asyncio.CancelledError:
                    pass
                self._loop_task = None
            self._running_trigger_ids.clear()
            self._monitored_run_ids.clear()

    async def reload(self) -> None:
        settings = self._get_settings()
        definitions = list_trigger_definitions(settings.config_dir)
        self._definitions = {definition.id: definition for definition in definitions}
        self._states = {
            definition.id: load_trigger_state(settings.data_dir, definition.id)
            for definition in definitions
        }
        for definition in definitions:
            state = self._states[definition.id]
            state.next_run_at = compute_next_run_at(definition, state)
            save_trigger_state(settings.data_dir, definition.id, state)

    async def list_triggers(self) -> list[dict[str, Any]]:
        await self.reload()
        return [
            serialize_trigger(definition, self._states.get(definition.id, TriggerState()))
            for definition in sorted(self._definitions.values(), key=lambda item: (item.protected is False, item.name.lower()))
        ]

    async def get_trigger(self, trigger_id: str) -> dict[str, Any] | None:
        await self.reload()
        definition = self._definitions.get(trigger_id)
        if definition is None:
            return None
        return serialize_trigger(definition, self._states.get(trigger_id, TriggerState()))

    async def emit_workspace_event(self, event_name: str, payload: Mapping[str, Any], *, dedupe_key: str | None = None) -> None:
        await self.reload()
        for definition in self._definitions.values():
            if definition.source_type != "workspace_event" or not definition.enabled:
                continue
            if str(definition.source.get("event_name") or "").strip() != event_name:
                continue
            await self._schedule_trigger_fire(definition, dict(payload), dedupe_key=dedupe_key or event_name)

    async def emit_flow_event(self, payload: Mapping[str, Any], *, dedupe_key: str) -> None:
        await self.reload()
        flow_name = str(payload.get("flow_name") or "").strip()
        status = str(payload.get("status") or "").strip().lower()
        for definition in self._definitions.values():
            if definition.source_type != "flow_event" or not definition.enabled:
                continue
            configured_flow_name = str(definition.source.get("flow_name") or "").strip()
            if configured_flow_name and configured_flow_name != flow_name:
                continue
            configured_statuses = definition.source.get("statuses")
            statuses = configured_statuses if isinstance(configured_statuses, list) else []
            if statuses and status not in statuses:
                continue
            await self._schedule_trigger_fire(definition, dict(payload), dedupe_key=dedupe_key)

    async def handle_webhook(
        self,
        *,
        webhook_key: str,
        webhook_secret: str,
        payload: Mapping[str, Any],
        request_id: str | None = None,
    ) -> dict[str, Any]:
        await self.reload()
        definition = get_trigger_by_webhook_key(self._get_settings().config_dir, webhook_key)
        if definition is None:
            raise TriggerError("Unknown webhook key.")
        if definition.source_type != "webhook":
            raise TriggerError("Webhook key does not resolve to a webhook trigger.")
        if not definition.enabled:
            raise TriggerError("Webhook trigger is disabled.")
        if not verify_webhook_secret(definition, webhook_secret):
            raise TriggerError("Webhook secret is invalid.")
        state = self._states.setdefault(definition.id, TriggerState())
        if request_id and request_id in state.dedupe_keys:
            return {"ok": True, "trigger_id": definition.id}
        if definition.id in self._running_trigger_ids:
            self._record_history(
                definition.id,
                status="skipped",
                message="Trigger is already running.",
                dedupe_key=request_id,
            )
            return {"ok": True, "trigger_id": definition.id}
        self._running_trigger_ids.add(definition.id)
        try:
            await self._execute_trigger(definition, dict(payload), dedupe_key=request_id)
        finally:
            self._running_trigger_ids.discard(definition.id)
        return {"ok": True, "trigger_id": definition.id}

    async def observe_run(self, *, run_id: str, flow_name: str, project_path: str | None) -> None:
        if not run_id or run_id in self._monitored_run_ids:
            return
        self._monitored_run_ids.add(run_id)
        asyncio.create_task(self._monitor_run(run_id=run_id, flow_name=flow_name, project_path=project_path))

    async def _run_loop(self) -> None:
        try:
            while True:
                try:
                    await self._process_due_triggers()
                except Exception as exc:  # noqa: BLE001
                    LOGGER.warning("Trigger runtime loop iteration failed: %s", exc)
                await asyncio.sleep(1.0)
        except asyncio.CancelledError:
            raise

    async def _process_due_triggers(self) -> None:
        await self.reload()
        now = _utc_now()
        for definition in self._definitions.values():
            if not definition.enabled:
                continue
            if definition.source_type == "schedule":
                await self._process_schedule_trigger(definition, now)
            elif definition.source_type == "poll":
                await self._process_poll_trigger(definition, now)

    async def _process_schedule_trigger(self, definition: TriggerDefinition, now: datetime) -> None:
        state = self._states.setdefault(definition.id, TriggerState())
        due_at = _schedule_due_at(definition.source, state, now)
        state.next_run_at = _compute_schedule_next_run_at(definition.source, state, now=now)
        save_trigger_state(self._get_settings().data_dir, definition.id, state)
        if due_at is None:
            return
        await self._schedule_trigger_fire(
            definition,
            {"scheduled_at": _datetime_to_iso(due_at)},
            dedupe_key=_datetime_to_iso(due_at),
        )

    async def _process_poll_trigger(self, definition: TriggerDefinition, now: datetime) -> None:
        state = self._states.setdefault(definition.id, TriggerState())
        next_run_at = _parse_optional_datetime(state.next_run_at)
        if next_run_at is not None and now < next_run_at:
            return
        interval_seconds = int(definition.source["interval_seconds"])
        state.next_run_at = _datetime_to_iso(now + timedelta(seconds=interval_seconds))
        save_trigger_state(self._get_settings().data_dir, definition.id, state)
        try:
            async with httpx.AsyncClient(timeout=10.0) as client:
                headers = definition.source.get("headers")
                response = await client.get(
                    str(definition.source["url"]),
                    headers=headers if isinstance(headers, dict) else None,
                )
            response.raise_for_status()
            payload = response.json()
            items = _extract_json_path(payload, str(definition.source["items_path"]))
            if not isinstance(items, list):
                raise TriggerError("Poll source items_path did not resolve to a JSON array.")
            for item in items:
                item_id = _extract_json_path(item, str(definition.source["item_id_path"]))
                if item_id is None:
                    continue
                dedupe_key = f"poll:{item_id}"
                if dedupe_key in state.dedupe_keys:
                    continue
                await self._schedule_trigger_fire(definition, {"poll_item": item}, dedupe_key=dedupe_key)
        except Exception as exc:  # noqa: BLE001
            self._record_failure(definition.id, f"Polling failed: {exc}")

    async def _schedule_trigger_fire(
        self,
        definition: TriggerDefinition,
        payload: dict[str, Any],
        *,
        dedupe_key: str | None,
    ) -> None:
        if definition.id in self._running_trigger_ids:
            self._record_history(
                definition.id,
                status="skipped",
                message="Trigger is already running.",
                dedupe_key=dedupe_key,
            )
            return
        state = self._states.setdefault(definition.id, TriggerState())
        if dedupe_key and dedupe_key in state.dedupe_keys:
            return
        self._running_trigger_ids.add(definition.id)
        asyncio.create_task(self._run_scheduled_trigger(definition, payload, dedupe_key=dedupe_key))

    async def _run_scheduled_trigger(
        self,
        definition: TriggerDefinition,
        payload: dict[str, Any],
        *,
        dedupe_key: str | None,
    ) -> None:
        try:
            await self._execute_trigger(definition, payload, dedupe_key=dedupe_key)
        finally:
            self._running_trigger_ids.discard(definition.id)

    async def _execute_trigger(
        self,
        definition: TriggerDefinition,
        payload: dict[str, Any],
        *,
        dedupe_key: str | None,
    ) -> None:
        try:
            run_id = await self._launch_trigger_flow(definition, payload)
            state = self._states.setdefault(definition.id, TriggerState())
            state.last_fired_at = _iso_now()
            state.last_result = "success"
            state.last_error = None
            if dedupe_key:
                state.dedupe_keys = _append_bounded(state.dedupe_keys, dedupe_key, limit=MAX_DEDUPE_KEYS)
                if dedupe_key.startswith("poll:"):
                    item_id = dedupe_key.split(":", 1)[1]
                    state.seen_item_ids = _append_bounded(state.seen_item_ids, item_id, limit=MAX_POLL_ITEM_IDS)
            state.next_run_at = compute_next_run_at(definition, state)
            save_trigger_state(self._get_settings().data_dir, definition.id, state)
            self._record_history(
                definition.id,
                status="success",
                message="Trigger fired successfully.",
                run_id=run_id,
                dedupe_key=dedupe_key,
            )
        except Exception as exc:  # noqa: BLE001
            self._record_failure(definition.id, str(exc), dedupe_key=dedupe_key)

    async def _launch_trigger_flow(self, definition: TriggerDefinition, payload: dict[str, Any]) -> str:
        action = definition.action
        working_directory = action.project_path or str(self._get_settings().data_dir)
        launch_context = {
            "context.trigger_static": dict(action.static_context),
            "context.trigger_payload": payload,
            "context.spark_trigger": {
                "trigger_id": definition.id,
                "trigger_name": definition.name,
                "source_type": definition.source_type,
            },
        }
        try:
            launch_payload = await self._get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=action.flow_name,
                working_directory=working_directory,
                model=None,
                launch_context=launch_context,
            )
        except AttractorApiError as exc:
            raise TriggerError(str(exc)) from exc
        if launch_payload.get("status") != "started":
            error = str(launch_payload.get("error") or "Trigger launch failed.")
            raise TriggerError(error)
        run_id = str(launch_payload.get("run_id") or "").strip()
        if not run_id:
            raise TriggerError("Trigger launch did not return a run id.")
        await self.observe_run(run_id=run_id, flow_name=action.flow_name, project_path=action.project_path)
        return run_id

    async def _monitor_run(self, *, run_id: str, flow_name: str, project_path: str | None) -> None:
        try:
            payload = await self._wait_for_run_terminal_status(run_id)
            status = str(payload.get("status", "")).strip().lower()
            await self.emit_flow_event(
                {
                    "run_id": run_id,
                    "flow_name": flow_name,
                    "project_path": project_path,
                    "status": status,
                },
                dedupe_key=f"flow:{run_id}",
            )
        except Exception as exc:  # noqa: BLE001
            LOGGER.warning("Failed to monitor trigger-observed run %s: %s", run_id, exc)
        finally:
            self._monitored_run_ids.discard(run_id)

    async def _wait_for_run_terminal_status(self, run_id: str) -> dict[str, Any]:
        client = self._get_attractor_client()
        while True:
            payload = await client.get_pipeline(run_id)
            status = str(payload.get("status", "")).strip().lower()
            if status in TERMINAL_PIPELINE_STATUSES:
                return payload
            await asyncio.sleep(1.0)

    def _record_failure(self, trigger_id: str, message: str, *, dedupe_key: str | None = None) -> None:
        state = self._states.setdefault(trigger_id, TriggerState())
        state.last_fired_at = _iso_now()
        state.last_result = "failed"
        state.last_error = message
        if dedupe_key:
            state.dedupe_keys = _append_bounded(state.dedupe_keys, dedupe_key, limit=MAX_DEDUPE_KEYS)
        save_trigger_state(self._get_settings().data_dir, trigger_id, state)
        self._record_history(trigger_id, status="failed", message=message, dedupe_key=dedupe_key)

    def _record_history(
        self,
        trigger_id: str,
        *,
        status: str,
        message: str,
        run_id: str | None = None,
        dedupe_key: str | None = None,
    ) -> None:
        state = self._states.setdefault(trigger_id, TriggerState())
        state.recent_history = [
            {
                "timestamp": _iso_now(),
                "status": status,
                "message": message,
                "run_id": run_id,
                "dedupe_key": dedupe_key,
            },
            *state.recent_history,
        ][:MAX_RECENT_HISTORY]
        save_trigger_state(self._get_settings().data_dir, trigger_id, state)


def _parse_trigger_definition(trigger_id: str, payload: dict[str, Any]) -> TriggerDefinition:
    action_payload = payload.get("action")
    source_payload = payload.get("source")
    if not isinstance(action_payload, dict) or not isinstance(source_payload, dict):
        raise TriggerError("Trigger definition must include [action] and [source] sections.")
    action = _normalize_action(action_payload)
    source_type = str(payload.get("source_type") or "").strip()
    normalized_source_payload: dict[str, Any] = {}
    for key, value in source_payload.items():
        if isinstance(key, str) and key.endswith("_json") and isinstance(value, str):
            try:
                normalized_source_payload[key[:-5]] = json.loads(value)
            except Exception as exc:  # noqa: BLE001
                raise TriggerError(f"Invalid JSON value for source.{key}") from exc
            continue
        normalized_source_payload[str(key)] = value
    source = _normalize_source(source_type, normalized_source_payload)
    return TriggerDefinition(
        id=trigger_id,
        name=str(payload.get("name") or "").strip(),
        enabled=bool(payload.get("enabled", True)),
        protected=bool(payload.get("protected", False)),
        source_type=source_type,
        action=action,
        source=source,
        created_at=str(payload.get("created_at") or ""),
        updated_at=str(payload.get("updated_at") or ""),
    )


def _normalize_action(payload: Mapping[str, Any]) -> TriggerAction:
    flow_name = str(payload.get("flow_name") or "").strip()
    if not flow_name:
        raise TriggerError("Trigger action requires a flow_name.")
    project_path_value = payload.get("project_path")
    project_path = normalize_project_path(str(project_path_value)) if isinstance(project_path_value, str) and project_path_value.strip() else None
    static_context = payload.get("static_context")
    if static_context is None and isinstance(payload.get("static_context_json"), str):
        try:
            static_context = json.loads(str(payload["static_context_json"]))
        except Exception as exc:  # noqa: BLE001
            raise TriggerError("Trigger action static_context_json must be valid JSON.") from exc
    if static_context is None:
        normalized_static_context: dict[str, Any] = {}
    elif isinstance(static_context, dict):
        normalized_static_context = dict(static_context)
    else:
        raise TriggerError("Trigger action static_context must be a JSON object.")
    return TriggerAction(
        flow_name=flow_name,
        project_path=project_path,
        static_context=normalized_static_context,
    )


def _normalize_source(
    source_type: str,
    payload: Mapping[str, Any],
    *,
    preserve_secret_hash: Any = None,
) -> dict[str, Any]:
    normalized_source_type = source_type.strip()
    if normalized_source_type not in TRIGGER_SOURCE_TYPES:
        raise TriggerError(f"Unsupported trigger source type: {source_type}")
    if normalized_source_type == "schedule":
        kind = str(payload.get("kind") or "").strip().lower()
        if kind not in {"once", "interval", "weekly"}:
            raise TriggerError("Schedule triggers require kind=once|interval|weekly.")
        if kind == "once":
            run_at = str(payload.get("run_at") or "").strip()
            if not run_at:
                raise TriggerError("One-shot schedule triggers require run_at.")
            return {"kind": kind, "run_at": run_at}
        if kind == "interval":
            interval_seconds = _coerce_positive_int(payload.get("interval_seconds"), "interval_seconds")
            return {"kind": kind, "interval_seconds": interval_seconds}
        weekdays_value = payload.get("weekdays")
        weekdays = [str(entry).strip().lower() for entry in weekdays_value] if isinstance(weekdays_value, list) else []
        if not weekdays or any(entry not in WEEKDAY_ORDER for entry in weekdays):
            raise TriggerError("Weekly schedule triggers require weekdays using mon..sun.")
        hour = _coerce_int_range(payload.get("hour"), "hour", minimum=0, maximum=23)
        minute = _coerce_int_range(payload.get("minute"), "minute", minimum=0, maximum=59)
        return {"kind": kind, "weekdays": sorted(set(weekdays), key=WEEKDAY_ORDER.index), "hour": hour, "minute": minute}
    if normalized_source_type == "poll":
        url = str(payload.get("url") or "").strip()
        if not url.startswith("http://") and not url.startswith("https://"):
            raise TriggerError("Poll triggers require an http(s) url.")
        interval_seconds = _coerce_positive_int(payload.get("interval_seconds"), "interval_seconds")
        items_path = str(payload.get("items_path") or "").strip()
        item_id_path = str(payload.get("item_id_path") or "").strip()
        if not items_path or not item_id_path:
            raise TriggerError("Poll triggers require items_path and item_id_path.")
        headers = payload.get("headers")
        normalized_headers = {str(key): str(value) for key, value in headers.items()} if isinstance(headers, dict) else {}
        return {
            "url": url,
            "interval_seconds": interval_seconds,
            "items_path": items_path,
            "item_id_path": item_id_path,
            "headers": normalized_headers,
        }
    if normalized_source_type == "webhook":
        webhook_key = str(payload.get("webhook_key") or "").strip()
        secret_hash = str(payload.get("secret_hash") or preserve_secret_hash or "").strip()
        if not webhook_key:
            raise TriggerError("Webhook triggers require webhook_key.")
        if not secret_hash:
            raise TriggerError("Webhook triggers require secret_hash.")
        return {"webhook_key": webhook_key, "secret_hash": secret_hash}
    if normalized_source_type == "workspace_event":
        event_name = str(payload.get("event_name") or "").strip()
        if event_name not in WORKSPACE_TRIGGER_EVENTS:
            raise TriggerError(f"Unsupported workspace event trigger: {event_name}")
        return {"event_name": event_name}
    flow_name = str(payload.get("flow_name") or "").strip() or None
    statuses_value = payload.get("statuses")
    statuses = [str(entry).strip().lower() for entry in statuses_value] if isinstance(statuses_value, list) else []
    normalized_statuses = [status for status in statuses if status in TERMINAL_PIPELINE_STATUSES]
    if statuses and not normalized_statuses:
        raise TriggerError("Flow-event triggers require terminal statuses when statuses are provided.")
    return {"flow_name": flow_name, "statuses": normalized_statuses}


def _schedule_due_at(source: Mapping[str, Any], state: TriggerState, now: datetime) -> datetime | None:
    kind = str(source.get("kind") or "").strip().lower()
    last_fired_at = _parse_optional_datetime(state.last_fired_at)
    if kind == "once":
        run_at = _parse_required_datetime(str(source["run_at"]))
        if last_fired_at is not None:
            return None
        if now >= run_at:
            return run_at
        return None
    if kind == "interval":
        interval_seconds = int(source["interval_seconds"])
        if last_fired_at is None:
            return now
        next_due = last_fired_at + timedelta(seconds=interval_seconds)
        return next_due if now >= next_due else None
    scheduled = _weekly_scheduled_time(source, now)
    last_history_key = state.dedupe_keys[0] if state.dedupe_keys else None
    schedule_key = f"weekly:{_datetime_to_iso(scheduled)}"
    if scheduled <= now and last_history_key != schedule_key:
        return scheduled
    return None


def _compute_schedule_next_run_at(source: Mapping[str, Any], state: TriggerState, *, now: datetime | None = None) -> str | None:
    current_time = now or _utc_now()
    kind = str(source.get("kind") or "").strip().lower()
    last_fired_at = _parse_optional_datetime(state.last_fired_at)
    if kind == "once":
        run_at = _parse_required_datetime(str(source["run_at"]))
        if last_fired_at is not None:
            return None
        return _datetime_to_iso(run_at)
    if kind == "interval":
        interval_seconds = int(source["interval_seconds"])
        base_time = last_fired_at or current_time
        return _datetime_to_iso(base_time + timedelta(seconds=interval_seconds))
    scheduled = _weekly_scheduled_time(source, current_time)
    if scheduled <= current_time:
        scheduled = _weekly_scheduled_time(source, current_time + timedelta(days=1))
    return _datetime_to_iso(scheduled)


def _weekly_scheduled_time(source: Mapping[str, Any], now: datetime) -> datetime:
    weekdays = list(source.get("weekdays") or [])
    hour = int(source.get("hour") or 0)
    minute = int(source.get("minute") or 0)
    for offset in range(0, 8):
        candidate = now + timedelta(days=offset)
        weekday = WEEKDAY_ORDER[candidate.weekday()]
        if weekday not in weekdays:
            continue
        scheduled = candidate.replace(hour=hour, minute=minute, second=0, microsecond=0)
        if offset > 0 or scheduled >= now.replace(second=0, microsecond=0):
            return scheduled
    return now.replace(hour=hour, minute=minute, second=0, microsecond=0)


def _extract_json_path(value: Any, path: str) -> Any:
    current: Any = value
    for part in [segment.strip() for segment in path.split(".") if segment.strip()]:
        if isinstance(current, dict):
            current = current.get(part)
            continue
        return None
    return current


def _append_bounded(values: list[str], value: str, *, limit: int) -> list[str]:
    deduped = [value, *[entry for entry in values if entry != value]]
    return deduped[:limit]


def _toml_source_line(key: str, value: Any) -> list[str]:
    if isinstance(value, bool):
        return [f"{key} = {_toml_bool(value)}"]
    if isinstance(value, int):
        return [f"{key} = {value}"]
    if isinstance(value, list):
        return [f"{key} = [{', '.join(_toml_string(str(entry)) for entry in value)}]"]
    if isinstance(value, dict):
        return [f'{key}_json = {_toml_string(json.dumps(value, sort_keys=True))}']
    return [f"{key} = {_toml_string(str(value))}"]


def _toml_string(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def _toml_bool(value: bool) -> str:
    return "true" if value else "false"


def _normalize_optional_string(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    trimmed = value.strip()
    return trimmed or None


def _coerce_positive_int(value: object, field_name: str) -> int:
    number = _coerce_int(value, field_name)
    if number <= 0:
        raise TriggerError(f"{field_name} must be greater than zero.")
    return number


def _coerce_int_range(value: object, field_name: str, *, minimum: int, maximum: int) -> int:
    number = _coerce_int(value, field_name)
    if number < minimum or number > maximum:
        raise TriggerError(f"{field_name} must be between {minimum} and {maximum}.")
    return number


def _coerce_int(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TriggerError(f"{field_name} must be an integer.")
    return value


def _iso_now() -> str:
    return _datetime_to_iso(_utc_now())


def _utc_now() -> datetime:
    return datetime.now(UTC)


def _datetime_to_iso(value: datetime) -> str:
    return value.astimezone(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _parse_required_datetime(value: str) -> datetime:
    parsed = _parse_optional_datetime(value)
    if parsed is None:
        raise TriggerError(f"Invalid timestamp: {value}")
    return parsed


def _parse_optional_datetime(value: str | None) -> datetime | None:
    if not value:
        return None
    text = value.strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)
