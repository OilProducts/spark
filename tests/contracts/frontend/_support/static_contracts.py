from __future__ import annotations

import re
from pathlib import Path


ALLOWED_COMPONENTS_APP_MODULES = {"dialog-controller", "flow-tree"}
ALLOWED_COMPONENTS_APP_FILES = {f"{module}.tsx" for module in ALLOWED_COMPONENTS_APP_MODULES}


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[4]


def _frontend_src_root() -> Path:
    return _repo_root() / "frontend" / "src"


def iter_runtime_source_files() -> list[Path]:
    return _collect_runtime_source_files(_frontend_src_root())


def _collect_runtime_source_files(directory_path: Path) -> list[Path]:
    files: list[Path] = []
    for entry_path in sorted(directory_path.iterdir()):
        if entry_path.name in {"__tests__", "test"}:
            continue
        if entry_path.is_dir():
            files.extend(_collect_runtime_source_files(entry_path))
            continue
        if entry_path.is_file() and entry_path.suffix in {".ts", ".tsx"}:
            files.append(entry_path)
    return files


def read_runtime_ui_source() -> str:
    return "\n".join(path.read_text(encoding="utf-8") for path in iter_runtime_source_files())


def legacy_ui_boundary_violations() -> list[str]:
    repo_root = _repo_root()
    violations: list[str] = []

    legacy_ui_directory = _frontend_src_root() / "ui"
    if legacy_ui_directory.exists():
        violations.append(f"{legacy_ui_directory.relative_to(repo_root)}: legacy src/ui directory still exists")

    for path in iter_runtime_source_files():
        source = path.read_text(encoding="utf-8")
        relative_path = path.relative_to(repo_root)
        if "@/ui" in source:
            violations.append(f"{relative_path}: contains legacy @/ui reference")
        if "frontend/src/ui" in source:
            violations.append(f"{relative_path}: contains legacy frontend/src/ui reference")

    return violations


def shared_app_boundary_violations() -> list[str]:
    repo_root = _repo_root()
    violations: list[str] = []
    components_app_directory = _frontend_src_root() / "components" / "app"

    if components_app_directory.exists():
        for path in sorted(components_app_directory.rglob("*.tsx")):
            if "__tests__" in path.parts:
                continue
            relative_to_app = path.relative_to(components_app_directory).as_posix()
            if relative_to_app not in ALLOWED_COMPONENTS_APP_FILES:
                violations.append(
                    f"{path.relative_to(repo_root)}: unexpected shared components/app module"
                )

    import_pattern = re.compile(r"@/components/app/([A-Za-z0-9_-]+)")
    for path in iter_runtime_source_files():
        source = path.read_text(encoding="utf-8")
        relative_path = path.relative_to(repo_root)
        for match in import_pattern.finditer(source):
            module_name = match.group(1)
            if module_name not in ALLOWED_COMPONENTS_APP_MODULES:
                violations.append(
                    f"{relative_path}: imports forbidden shared app module {module_name}"
                )

    return violations


REQUIRED_UI_ENDPOINT_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("/attractor/api/flows", re.compile(r"fetch\(\s*['\"]/attractor/api/flows['\"]|fetchFlowListValidated\(")),
    (
        "/attractor/api/flows/{name}",
        re.compile(r"fetch\(\s*`/attractor/api/flows/\$\{encodeURIComponent\([^)]+\)\}`|fetchFlowPayloadValidated\("),
    ),
    ("/workspace/api/flows", re.compile(r"fetchWorkspaceFlowListValidated\(|fetch\(\s*`?/workspace/api/flows\?surface=")),
    (
        "/workspace/api/flows/{flow_name}",
        re.compile(r"fetchWorkspaceFlowValidated\(|fetch\(\s*`?/workspace/api/flows/\$\{encodeURIComponent\([^)]+\)\}\?surface="),
    ),
    (
        "/workspace/api/flows/{flow_name}/raw",
        re.compile(r"fetchWorkspaceFlowRawValidated\(|fetch\(\s*`?/workspace/api/flows/\$\{encodeURIComponent\([^)]+\)\}/raw\?surface="),
    ),
    (
        "/workspace/api/flows/{flow_name}/launch-policy",
        re.compile(r"updateWorkspaceFlowLaunchPolicyValidated\(|fetch\(\s*`?/workspace/api/flows/\$\{encodeURIComponent\([^)]+\)\}/launch-policy`"),
    ),
    ("/workspace/api/conversations/{id}", re.compile(r"fetchConversationSnapshotValidated\(")),
    ("/workspace/api/conversations/{id} (DELETE)", re.compile(r"deleteConversationValidated\(")),
    ("/workspace/api/projects/pick-directory", re.compile(r"pickProjectDirectoryValidated\(")),
    (
        "/workspace/api/conversations/{id}/events",
        re.compile(r"new EventSource\(\s*eventStreamUrl\s*\)|/workspace/api/conversations/\$\{encodeURIComponent\([^)]+\)\}/events\?project_path="),
    ),
    ("/workspace/api/conversations/{id}/turns", re.compile(r"sendConversationTurnValidated\(")),
    ("/attractor/preview", re.compile(r"fetch\(\s*['\"]/attractor/preview['\"]|fetchPreviewValidated\(")),
    ("/attractor/pipelines", re.compile(r"fetch\(\s*['\"]/attractor/pipelines['\"]|fetchPipelineStartValidated\(")),
    (
        "/attractor/pipelines/{id}/continue",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/continue`|fetchPipelineContinueValidated\("),
    ),
    (
        "/attractor/pipelines/{id}",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}`\s*(?:,|\))|fetchPipelineStatusValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/events",
        re.compile(r"pipelineEventsUrl(?:WithAfterSequence)?\(|new EventSource\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/events`"),
    ),
    (
        "/attractor/pipelines/{id}/journal",
        re.compile(r"pipelineJournalUrl\(|fetchPipelineJournalValidated\("),
    ),
    (
        "/attractor/runs/events",
        re.compile(r"runsEventsUrl\(|new EventSource\(\s*`?/attractor/runs/events"),
    ),
    (
        "/attractor/pipelines/{id}/cancel",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/cancel`\s*,\s*\{\s*method:\s*['\"]POST['\"]|fetchPipelineCancelValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/graph",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/graph`|fetchPipelineGraphValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/graph-preview",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/graph-preview`|fetchPipelineGraphPreviewValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/questions",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/questions`\s*(?:,|\))|fetchPipelineQuestionsValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/questions/{qid}/answer",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/questions/\$\{encodeURIComponent\([^)]+\)\}/answer`|fetchPipelineAnswerValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/checkpoint",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/checkpoint`|fetchPipelineCheckpointValidated\("),
    ),
    (
        "/attractor/pipelines/{id}/context",
        re.compile(r"fetch\(\s*`/attractor/pipelines/\$\{encodeURIComponent\([^)]+\)\}/context`|fetchPipelineContextValidated\("),
    ),
    ("/attractor/runs", re.compile(r"fetch\(\s*['\"]/attractor/runs['\"]|fetchRunsListValidated\(")),
    ("/attractor/status", re.compile(r"fetch\(\s*['\"]/attractor/status['\"]|fetchRuntimeStatusValidated\(")),
)


def missing_required_ui_endpoints() -> list[str]:
    runtime_source = read_runtime_ui_source()
    return [
        endpoint
        for endpoint, pattern in REQUIRED_UI_ENDPOINT_PATTERNS
        if pattern.search(runtime_source) is None
    ]


def read_frontend_index_css() -> str:
    index_css_path = _frontend_src_root() / "index.css"
    return index_css_path.read_text(encoding="utf-8")


def parse_root_hsl_token(css_source: str, token_name: str) -> tuple[float, float, float]:
    token_pattern = re.compile(rf"--{re.escape(token_name)}:\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%\s*;")
    token_match = token_pattern.search(css_source)
    if token_match is None:
        raise AssertionError(f"Unable to find --{token_name} token in frontend index.css")
    return tuple(float(token_match.group(index)) for index in range(1, 4))


def hsl_to_rgb(color: tuple[float, float, float]) -> tuple[int, int, int]:
    hue, saturation, lightness = color
    normalized_hue = ((hue % 360) + 360) % 360
    normalized_saturation = max(0.0, min(100.0, saturation)) / 100
    normalized_lightness = max(0.0, min(100.0, lightness)) / 100
    chroma = (1 - abs((2 * normalized_lightness) - 1)) * normalized_saturation
    hue_segment = normalized_hue / 60
    secondary = chroma * (1 - abs((hue_segment % 2) - 1))

    red_prime = 0.0
    green_prime = 0.0
    blue_prime = 0.0
    if 0 <= hue_segment < 1:
        red_prime = chroma
        green_prime = secondary
    elif 1 <= hue_segment < 2:
        red_prime = secondary
        green_prime = chroma
    elif 2 <= hue_segment < 3:
        green_prime = chroma
        blue_prime = secondary
    elif 3 <= hue_segment < 4:
        green_prime = secondary
        blue_prime = chroma
    elif 4 <= hue_segment < 5:
        red_prime = secondary
        blue_prime = chroma
    else:
        red_prime = chroma
        blue_prime = secondary

    match = normalized_lightness - chroma / 2

    def to_byte(channel: float) -> int:
        return round((channel + match) * 255)

    return (to_byte(red_prime), to_byte(green_prime), to_byte(blue_prime))


def blend_on_white(color: tuple[int, int, int], alpha: float) -> tuple[int, int, int]:
    normalized_alpha = max(0.0, min(1.0, alpha))

    def blend_channel(channel: int) -> int:
        return round((normalized_alpha * channel) + ((1 - normalized_alpha) * 255))

    return tuple(blend_channel(channel) for channel in color)


def contrast_ratio(color_a: tuple[int, int, int], color_b: tuple[int, int, int]) -> float:
    def to_linear(channel: int) -> float:
        srgb = channel / 255
        return srgb / 12.92 if srgb <= 0.03928 else ((srgb + 0.055) / 1.055) ** 2.4

    def luminance(color: tuple[int, int, int]) -> float:
        red, green, blue = color
        return (0.2126 * to_linear(red)) + (0.7152 * to_linear(green)) + (0.0722 * to_linear(blue))

    lighter = max(luminance(color_a), luminance(color_b))
    darker = min(luminance(color_a), luminance(color_b))
    return (lighter + 0.05) / (darker + 0.05)
