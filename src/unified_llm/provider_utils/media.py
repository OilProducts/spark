from __future__ import annotations

import base64
import logging
import mimetypes
import os
from dataclasses import replace
from pathlib import Path
from typing import Any

from ..errors import ConfigurationError, InvalidRequestError
from ..types import AudioData, DocumentData, ImageData

logger = logging.getLogger(__name__)


def is_local_media_path(value: str) -> bool:
    return value.startswith(("/", "./", "~"))


def _guess_media_type(path: str | os.PathLike[str], default: str | None = None) -> str | None:
    guessed, _ = mimetypes.guess_type(os.fspath(path))
    if guessed is not None:
        return guessed
    return default


def _read_local_file(path: str | os.PathLike[str], *, kind: str) -> bytes:
    resolved_path = Path(path).expanduser()
    try:
        return resolved_path.read_bytes()
    except OSError as error:
        logger.exception("Unable to read local %s input from %s", kind, resolved_path)
        raise InvalidRequestError(
            f"unable to read local {kind} input from {resolved_path}",
        ) from error


def _normalize_provider_name(provider: str | None, provider_name: str | None = None) -> str:
    resolved = provider if provider is not None else provider_name
    if resolved is None:
        raise TypeError("provider must be provided")
    if not isinstance(resolved, str):
        raise TypeError("provider must be a string")
    return resolved.casefold()


def normalize_image_input(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> ImageData:
    if isinstance(value, ImageData):
        image = value
        if image.url is not None and is_local_media_path(image.url):
            local_path = Path(image.url).expanduser()
            data = _read_local_file(local_path, kind="image")
            resolved_media_type = (
                image.media_type
                or media_type
                or _guess_media_type(local_path, default="image/png")
            )
            return ImageData(data=data, media_type=resolved_media_type)
        if image.data is not None:
            if media_type is None or image.media_type == media_type:
                return image
            return replace(image, media_type=media_type)
        if media_type is None or image.media_type == media_type:
            return image
        return replace(image, media_type=media_type)

    if isinstance(value, (bytes, bytearray)):
        return ImageData(data=bytes(value), media_type=media_type or "image/png")

    if isinstance(value, (str, os.PathLike)):
        text = os.fspath(value)
        if is_local_media_path(text):
            local_path = Path(text).expanduser()
            data = _read_local_file(local_path, kind="image")
            resolved_media_type = media_type or _guess_media_type(
                local_path,
                default="image/png",
            )
            return ImageData(data=data, media_type=resolved_media_type)
        return ImageData(url=text, media_type=media_type)

    raise TypeError("image input must be an ImageData, bytes, or string/path value")


def _image_source_data_uri(image: ImageData) -> str:
    if image.data is None:
        if image.url is None:
            raise TypeError("image must contain url or data")
        return image.url
    media_type = image.media_type or "image/png"
    encoded = base64.b64encode(image.data).decode("ascii")
    return f"data:{media_type};base64,{encoded}"


def prepare_openai_image_input(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> str:
    image = normalize_image_input(value, media_type=media_type)
    return _image_source_data_uri(image)


def prepare_anthropic_image_input(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> dict[str, Any]:
    image = normalize_image_input(value, media_type=media_type)
    if image.url is not None and image.data is None:
        return {"type": "url", "url": image.url}
    if image.data is None:
        raise TypeError("image must contain url or data")
    media_type_value = image.media_type or "image/png"
    encoded = base64.b64encode(image.data).decode("ascii")
    return {"type": "base64", "media_type": media_type_value, "data": encoded}


def prepare_gemini_image_input(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> dict[str, Any]:
    image = normalize_image_input(value, media_type=media_type)
    media_type_value = image.media_type or _guess_media_type(image.url or "", default="image/png")
    if image.url is not None and image.data is None:
        return {"fileUri": image.url, "mimeType": media_type_value}
    if image.data is None:
        raise TypeError("image must contain url or data")
    encoded = base64.b64encode(image.data).decode("ascii")
    return {"data": encoded, "mimeType": media_type_value}


def prepare_image_input(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    provider: str | None = None,
    provider_name: str | None = None,
    media_type: str | None = None,
) -> str | dict[str, Any]:
    provider_key = _normalize_provider_name(provider, provider_name)
    if provider_key == "openai":
        return prepare_openai_image_input(value, media_type=media_type)
    if provider_key == "anthropic":
        return prepare_anthropic_image_input(value, media_type=media_type)
    if provider_key == "gemini":
        return prepare_gemini_image_input(value, media_type=media_type)

    logger.warning("Unsupported provider for image preparation: %s", provider_key)
    raise ConfigurationError(f"Unsupported provider {provider_key!r} for image preparation")


def prepare_openai_image_block(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> dict[str, Any]:
    image = normalize_image_input(value, media_type=media_type)
    block = {
        "type": "input_image",
        "image_url": _image_source_data_uri(image),
    }
    if image.detail is not None:
        block["detail"] = image.detail
    return block


def prepare_anthropic_image_block(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> dict[str, Any]:
    return {"type": "image", "source": prepare_anthropic_image_input(value, media_type=media_type)}


def prepare_gemini_image_block(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> dict[str, Any]:
    source = prepare_gemini_image_input(value, media_type=media_type)
    if "data" in source:
        return {"inlineData": source}
    return {"fileData": source}


def prepare_image_block(
    value: ImageData | str | bytes | bytearray | os.PathLike[str],
    *,
    provider: str | None = None,
    provider_name: str | None = None,
    media_type: str | None = None,
) -> dict[str, Any]:
    provider_key = _normalize_provider_name(provider, provider_name)
    if provider_key == "openai":
        return prepare_openai_image_block(value, media_type=media_type)
    if provider_key == "anthropic":
        return prepare_anthropic_image_block(value, media_type=media_type)
    if provider_key == "gemini":
        return prepare_gemini_image_block(value, media_type=media_type)
    logger.warning("Unsupported provider for image block preparation: %s", provider_key)
    raise ConfigurationError(f"Unsupported provider {provider_key!r} for image preparation")


def _normalize_audio_input(
    value: AudioData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> AudioData:
    if isinstance(value, AudioData):
        audio = value
        if audio.url is not None and is_local_media_path(audio.url):
            local_path = Path(audio.url).expanduser()
            data = _read_local_file(local_path, kind="audio")
            resolved_media_type = audio.media_type or media_type or _guess_media_type(local_path)
            return AudioData(data=data, media_type=resolved_media_type)
        if media_type is None or audio.media_type == media_type:
            return audio
        return replace(audio, media_type=media_type)

    if isinstance(value, (bytes, bytearray)):
        return AudioData(data=bytes(value), media_type=media_type)

    if isinstance(value, (str, os.PathLike)):
        text = os.fspath(value)
        if is_local_media_path(text):
            local_path = Path(text).expanduser()
            data = _read_local_file(local_path, kind="audio")
            resolved_media_type = media_type or _guess_media_type(local_path)
            return AudioData(data=data, media_type=resolved_media_type)
        return AudioData(url=text, media_type=media_type)

    raise TypeError("audio input must be an AudioData, bytes, or string/path value")


def normalize_audio_input(
    value: AudioData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
) -> AudioData:
    return _normalize_audio_input(value, media_type=media_type)


def prepare_audio_input(
    value: AudioData | str | bytes | bytearray | os.PathLike[str],
    *,
    provider: str | None = None,
    provider_name: str | None = None,
    supported: bool = False,
    media_type: str | None = None,
) -> AudioData:
    audio = _normalize_audio_input(value, media_type=media_type)
    if supported:
        return audio

    provider_key = provider if provider is not None else provider_name
    logger.warning(
        "Provider %s does not support audio inputs",
        provider_key or "unknown provider",
    )
    raise InvalidRequestError(
        "audio inputs are not supported by this provider",
        provider=provider_key,
    )


def _normalize_document_input(
    value: DocumentData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
    file_name: str | None = None,
) -> DocumentData:
    if isinstance(value, DocumentData):
        document = value
        if document.url is not None and is_local_media_path(document.url):
            local_path = Path(document.url).expanduser()
            data = _read_local_file(local_path, kind="document")
            resolved_media_type = document.media_type or media_type or _guess_media_type(local_path)
            resolved_file_name = document.file_name or file_name or local_path.name
            return DocumentData(
                data=data,
                media_type=resolved_media_type,
                file_name=resolved_file_name,
            )
        if media_type is None and file_name is None:
            return document
        return replace(
            document,
            media_type=document.media_type if media_type is None else media_type,
            file_name=document.file_name if file_name is None else file_name,
        )

    if isinstance(value, (bytes, bytearray)):
        return DocumentData(
            data=bytes(value),
            media_type=media_type,
            file_name=file_name,
        )

    if isinstance(value, (str, os.PathLike)):
        text = os.fspath(value)
        if is_local_media_path(text):
            local_path = Path(text).expanduser()
            data = _read_local_file(local_path, kind="document")
            resolved_media_type = media_type or _guess_media_type(local_path)
            resolved_file_name = file_name or local_path.name
            return DocumentData(
                data=data,
                media_type=resolved_media_type,
                file_name=resolved_file_name,
            )
        return DocumentData(url=text, media_type=media_type, file_name=file_name)

    raise TypeError("document input must be a DocumentData, bytes, or string/path value")


def normalize_document_input(
    value: DocumentData | str | bytes | bytearray | os.PathLike[str],
    *,
    media_type: str | None = None,
    file_name: str | None = None,
) -> DocumentData:
    return _normalize_document_input(value, media_type=media_type, file_name=file_name)


def prepare_document_input(
    value: DocumentData | str | bytes | bytearray | os.PathLike[str],
    *,
    provider: str | None = None,
    provider_name: str | None = None,
    supported: bool = False,
    media_type: str | None = None,
    file_name: str | None = None,
) -> DocumentData:
    document = _normalize_document_input(
        value,
        media_type=media_type,
        file_name=file_name,
    )
    if supported:
        return document

    provider_key = provider if provider is not None else provider_name
    logger.warning(
        "Provider %s does not support document inputs",
        provider_key or "unknown provider",
    )
    raise InvalidRequestError(
        "document inputs are not supported by this provider",
        provider=provider_key,
    )


__all__ = [
    "is_local_media_path",
    "normalize_audio_input",
    "normalize_document_input",
    "normalize_image_input",
    "prepare_anthropic_image_block",
    "prepare_anthropic_image_input",
    "prepare_audio_input",
    "prepare_document_input",
    "prepare_gemini_image_block",
    "prepare_gemini_image_input",
    "prepare_image_block",
    "prepare_image_input",
    "prepare_openai_image_block",
    "prepare_openai_image_input",
]
