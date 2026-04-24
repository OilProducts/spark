from __future__ import annotations

import os
from collections.abc import Iterable, Iterator, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from unified_llm.models import get_model_info

from .environment import ExecutionEnvironment

PROJECT_INSTRUCTION_BYTE_BUDGET = 32 * 1024
PROJECT_INSTRUCTION_TRUNCATION_MARKER = "[Project instructions truncated at 32KB]"


def _validate_text(value: Any, field_name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{field_name} must be a string")
    return value


def _normalize_doc_path(path: Any) -> str:
    return str(path).replace("\\", "/")


def _working_directory_text(environment: ExecutionEnvironment) -> str:
    return _normalize_doc_path(environment.working_directory())


def _working_directory_path(environment: ExecutionEnvironment) -> Path:
    return Path(environment.working_directory()).expanduser().resolve(strict=False)


def _resolve_path_text(path_text: str) -> Path:
    return Path(path_text).expanduser().resolve(strict=False)


def _exec_command_output(
    environment: ExecutionEnvironment,
    command: str,
    working_directory: str,
) -> str | None:
    try:
        result = environment.exec_command(command, working_dir=working_directory)
    except Exception:
        return None

    if result.exit_code != 0:
        return None
    output = result.stdout.strip()
    return output or None


def _exec_command_candidates(
    environment: ExecutionEnvironment,
    command: str,
    working_directory: str,
) -> str | None:
    candidates: list[str] = []
    for candidate in (working_directory, str(_resolve_path_text(working_directory))):
        if candidate not in candidates:
            candidates.append(candidate)

    for candidate in candidates:
        output = _exec_command_output(environment, command, candidate)
        if output is not None:
            return output
    return None


def _safe_file_exists(environment: ExecutionEnvironment, path_text: str) -> bool:
    try:
        return bool(environment.file_exists(path_text))
    except Exception:
        return False


def _safe_read_file(environment: ExecutionEnvironment, path_text: str) -> str | None:
    try:
        return environment.read_file(path_text)
    except Exception:
        return None


def _provider_family(profile: Any) -> str | None:
    model = getattr(profile, "model", None)
    if isinstance(model, str) and model:
        model_info = get_model_info(model)
        if model_info is not None:
            return model_info.provider.casefold()

    probe_values = [
        getattr(profile, "id", None),
        getattr(profile, "model", None),
        getattr(profile, "display_name", None),
    ]
    haystack = " ".join(value for value in probe_values if isinstance(value, str)).casefold()
    if "anthropic" in haystack or "claude" in haystack:
        return "anthropic"
    if "gemini" in haystack:
        return "gemini"
    if "openai" in haystack or haystack.startswith(("gpt-", "o1", "o3")):
        return "openai"
    return None


def _recognized_filenames(provider_family: str | None) -> tuple[str, ...]:
    provider_specific = {
        "openai": ".codex/instructions.md",
        "anthropic": "CLAUDE.md",
        "gemini": "GEMINI.md",
    }.get(provider_family)
    if provider_specific is None:
        return ("AGENTS.md",)
    return ("AGENTS.md", provider_specific)


def _path_chain(root: Path, working_directory: Path) -> list[Path]:
    try:
        relative = working_directory.relative_to(root)
    except ValueError:
        return [working_directory]

    directories = [root]
    current = root
    for part in [part for part in relative.parts if part not in ("", ".")]:
        current = current / part
        directories.append(current)
    return directories


def _display_path(root: Path, path: Path) -> str:
    try:
        relative = path.relative_to(root)
    except ValueError:
        return path.as_posix()
    relative_text = relative.as_posix()
    return "." if relative_text == "." else relative_text


def _candidate_path_texts(target: Path, working_directory: Path) -> list[str]:
    relative_text = os.path.relpath(target, start=working_directory)
    absolute_text = str(target)
    candidates = []
    for candidate in (relative_text, absolute_text):
        normalized = _normalize_doc_path(candidate)
        if normalized not in candidates:
            candidates.append(normalized)
    return candidates


def _truncate_text_to_byte_budget(text: str, remaining_bytes: int) -> str:
    if remaining_bytes <= 0:
        return ""
    encoded = text.encode("utf-8")
    if len(encoded) <= remaining_bytes:
        return text

    lower = 0
    upper = len(text)
    while lower < upper:
        midpoint = (lower + upper + 1) // 2
        if len(text[:midpoint].encode("utf-8")) <= remaining_bytes:
            lower = midpoint
        else:
            upper = midpoint - 1
    return text[:lower]


@dataclass(slots=True)
class ProjectDocument:
    path: str
    content: str
    truncated: bool = False

    def __post_init__(self) -> None:
        self.path = _validate_text(self.path, "path")
        self.content = _validate_text(self.content, "content")
        if not isinstance(self.truncated, bool):
            raise TypeError("truncated must be a boolean")


@dataclass(slots=True)
class ProjectDocuments(Sequence[ProjectDocument]):
    documents: list[ProjectDocument] = field(default_factory=list)
    truncated: bool = False

    def __post_init__(self) -> None:
        self.documents = list(self.documents)
        if not isinstance(self.truncated, bool):
            raise TypeError("truncated must be a boolean")
        for document in self.documents:
            if not isinstance(document, ProjectDocument):
                raise TypeError("documents must contain ProjectDocument instances")

    def __iter__(self) -> Iterator[ProjectDocument]:
        return iter(self.documents)

    def __len__(self) -> int:
        return len(self.documents)

    def __getitem__(self, index: int) -> ProjectDocument:
        return self.documents[index]


def _apply_budget(
    documents: Iterable[ProjectDocument],
    *,
    budget_bytes: int = PROJECT_INSTRUCTION_BYTE_BUDGET,
) -> ProjectDocuments:
    if budget_bytes < 0:
        raise ValueError("budget_bytes must be non-negative")

    loaded_documents: list[ProjectDocument] = []
    remaining_bytes = budget_bytes
    truncated = False
    source_documents = list(documents)

    for document in source_documents:
        if document.truncated:
            loaded_documents.append(document)
            truncated = True
            continue

        if remaining_bytes <= 0:
            truncated = True
            break

        encoded_size = len(document.content.encode("utf-8"))
        if encoded_size <= remaining_bytes:
            loaded_documents.append(document)
            remaining_bytes -= encoded_size
            continue

        truncated_content = _truncate_text_to_byte_budget(document.content, remaining_bytes)
        loaded_documents.append(
            ProjectDocument(
                path=document.path,
                content=truncated_content,
                truncated=True,
            )
        )
        truncated = True
        break

    if not truncated and any(document.truncated for document in source_documents):
        truncated = True

    return ProjectDocuments(documents=loaded_documents, truncated=truncated)


def _collect_project_documents(
    environment: ExecutionEnvironment,
    profile: Any | None,
) -> list[ProjectDocument]:
    working_directory_text = _working_directory_text(environment)
    working_directory = _working_directory_path(environment)
    git_root_text = _exec_command_candidates(
        environment,
        "git rev-parse --show-toplevel",
        working_directory_text,
    )
    root = _resolve_path_text(git_root_text) if git_root_text is not None else working_directory
    provider_family = _provider_family(profile)
    filenames = _recognized_filenames(provider_family)
    directories = _path_chain(root, working_directory)

    documents: list[ProjectDocument] = []
    for directory in directories:
        for filename in filenames:
            target = directory / filename
            for candidate_text in _candidate_path_texts(target, working_directory):
                if not _safe_file_exists(environment, candidate_text):
                    continue
                content = _safe_read_file(environment, candidate_text)
                if content is None:
                    continue
                documents.append(
                    ProjectDocument(
                        path=_display_path(root, target),
                        content=content,
                    )
                )
                break
    return documents


def discover_project_documents(
    environment: ExecutionEnvironment,
    profile: Any | None = None,
    *,
    budget_bytes: int = PROJECT_INSTRUCTION_BYTE_BUDGET,
) -> ProjectDocuments:
    return _apply_budget(
        _collect_project_documents(environment, profile),
        budget_bytes=budget_bytes,
    )


def normalize_project_documents(
    project_documents: ProjectDocument
    | ProjectDocuments
    | Mapping[str, Any]
    | Iterable[ProjectDocument | tuple[str, Any]],
    *,
    budget_bytes: int = PROJECT_INSTRUCTION_BYTE_BUDGET,
) -> ProjectDocuments:
    if isinstance(project_documents, ProjectDocument):
        return ProjectDocuments(
            documents=[project_documents],
            truncated=project_documents.truncated,
        )
    if isinstance(project_documents, ProjectDocuments):
        return ProjectDocuments(
            documents=[
                ProjectDocument(
                    path=document.path,
                    content=document.content,
                    truncated=document.truncated,
                )
                for document in project_documents.documents
            ],
            truncated=project_documents.truncated,
        )

    raw_documents: list[ProjectDocument] = []
    if isinstance(project_documents, Mapping):
        for path, content in project_documents.items():
            raw_documents.append(
                ProjectDocument(path=_normalize_doc_path(path), content=str(content))
            )
    else:
        for item in project_documents:
            if isinstance(item, ProjectDocument):
                raw_documents.append(
                    ProjectDocument(
                        path=item.path,
                        content=item.content,
                        truncated=item.truncated,
                    )
                )
                continue
            try:
                path, content = item
            except Exception as exc:
                raise TypeError(
                    "project documents must be ProjectDocument instances or path/content pairs"
                ) from exc
            raw_documents.append(
                ProjectDocument(path=_normalize_doc_path(path), content=str(content))
            )

    if any(document.truncated for document in raw_documents):
        return ProjectDocuments(documents=raw_documents, truncated=True)
    return _apply_budget(raw_documents, budget_bytes=budget_bytes)


def render_project_documents(
    project_documents: ProjectDocument
    | ProjectDocuments
    | Mapping[str, Any]
    | Iterable[ProjectDocument | tuple[str, Any]],
    *,
    budget_bytes: int = PROJECT_INSTRUCTION_BYTE_BUDGET,
) -> str:
    bundle = normalize_project_documents(
        project_documents,
        budget_bytes=budget_bytes,
    )
    if len(bundle) == 0:
        return ""

    sections = [
        f"### {document.path}\n{document.content}"
        for document in bundle.documents
    ]
    if bundle.truncated or any(document.truncated for document in bundle.documents):
        sections.append(PROJECT_INSTRUCTION_TRUNCATION_MARKER)
    return "\n\n".join(sections)


def load_project_documents(
    environment: ExecutionEnvironment,
    profile: Any | None = None,
    *,
    budget_bytes: int = PROJECT_INSTRUCTION_BYTE_BUDGET,
) -> str:
    return render_project_documents(
        discover_project_documents(
            environment,
            profile,
            budget_bytes=budget_bytes,
        )
    )


__all__ = [
    "PROJECT_INSTRUCTION_BYTE_BUDGET",
    "PROJECT_INSTRUCTION_TRUNCATION_MARKER",
    "ProjectDocument",
    "ProjectDocuments",
    "discover_project_documents",
    "load_project_documents",
    "normalize_project_documents",
    "render_project_documents",
]
