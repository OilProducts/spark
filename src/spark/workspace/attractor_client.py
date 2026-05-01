from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional

import httpx


class AttractorApiError(RuntimeError):
    def __init__(self, message: str, *, status_code: int = 500) -> None:
        super().__init__(message)
        self.status_code = status_code


@dataclass(frozen=True)
class AttractorApiClient:
    base_url: str
    app: Any | None = None

    async def flow_exists(self, flow_name: str) -> bool:
        response = await self._request("GET", f"/api/flows/{flow_name}")
        return response.status_code == 200

    async def start_pipeline(
        self,
        *,
        run_id: Optional[str],
        flow_name: str,
        working_directory: str,
        model: Optional[str],
        llm_provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        execution_profile_id: Optional[str] = None,
        project_default_execution_profile_id: Optional[str] = None,
        goal: Optional[str] = None,
        launch_context: dict[str, Any] | None = None,
        spec_id: Optional[str] = None,
        plan_id: Optional[str] = None,
    ) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "run_id": run_id,
            "flow_name": flow_name,
            "working_directory": working_directory,
            "model": model,
            "goal": goal,
            "launch_context": launch_context,
            "spec_id": spec_id,
            "plan_id": plan_id,
        }
        if llm_provider is not None:
            payload["llm_provider"] = llm_provider
        if llm_profile is not None:
            payload["llm_profile"] = llm_profile
        if reasoning_effort is not None:
            payload["reasoning_effort"] = reasoning_effort
        if execution_profile_id is not None:
            payload["execution_profile_id"] = execution_profile_id
        if project_default_execution_profile_id is not None:
            payload["project_default_execution_profile_id"] = project_default_execution_profile_id
        return await self._request_json("POST", "/pipelines", json=payload)

    async def preview_flow(self, flow_content: str) -> dict[str, Any]:
        return await self._request_json("POST", "/preview", json={"flow_content": flow_content})

    async def get_pipeline(self, run_id: str) -> dict[str, Any]:
        return await self._request_json("GET", f"/pipelines/{run_id}")

    async def update_pipeline_metadata(
        self,
        run_id: str,
        *,
        spec_id: Optional[str] = None,
        plan_id: Optional[str] = None,
    ) -> dict[str, Any]:
        return await self._request_json(
            "PATCH",
            f"/pipelines/{run_id}/metadata",
            json={
                "spec_id": spec_id,
                "plan_id": plan_id,
            },
        )

    async def get_artifact_text(self, run_id: str, artifact_path: str) -> str:
        response = await self._request("GET", f"/pipelines/{run_id}/artifacts/{artifact_path}")
        if response.status_code >= 400:
            raise self._to_error(response)
        return response.text

    async def _request_json(self, method: str, path: str, **kwargs: Any) -> dict[str, Any]:
        response = await self._request(method, path, **kwargs)
        if response.status_code >= 400:
            raise self._to_error(response)
        payload = response.json()
        if not isinstance(payload, dict):
            raise AttractorApiError(f"Expected JSON object from Attractor for {path}.", status_code=500)
        return payload

    async def _request(self, method: str, path: str, **kwargs: Any) -> httpx.Response:
        async with self._client() as client:
            return await client.request(method, path, **kwargs)

    def _client(self) -> httpx.AsyncClient:
        if self.app is not None:
            return httpx.AsyncClient(transport=httpx.ASGITransport(app=self.app), base_url=self.base_url)
        return httpx.AsyncClient(base_url=self.base_url)

    def _to_error(self, response: httpx.Response) -> AttractorApiError:
        detail = ""
        try:
            payload = response.json()
            if isinstance(payload, dict):
                detail = str(payload.get("detail", "") or payload.get("error", "")).strip()
        except Exception:
            detail = response.text.strip()
        message = detail or f"Attractor request failed with status {response.status_code}."
        return AttractorApiError(message, status_code=response.status_code)
