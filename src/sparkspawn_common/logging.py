from __future__ import annotations

import logging
import sys


_SPARKSPAWN_LOGGER_NAME = "sparkspawn"
_SPARKSPAWN_STDOUT_HANDLER_NAME = "sparkspawn-stdout"


def configure_sparkspawn_logging(level: int = logging.INFO) -> logging.Logger:
    logger = logging.getLogger(_SPARKSPAWN_LOGGER_NAME)
    if not any(getattr(handler, "name", "") == _SPARKSPAWN_STDOUT_HANDLER_NAME for handler in logger.handlers):
        handler = logging.StreamHandler(sys.stdout)
        handler.set_name(_SPARKSPAWN_STDOUT_HANDLER_NAME)
        handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s"))
        logger.addHandler(handler)
    logger.setLevel(level)
    logger.propagate = False
    return logger


def get_sparkspawn_logger(name: str) -> logging.Logger:
    configure_sparkspawn_logging()
    normalized_name = name.strip()
    if not normalized_name:
        return logging.getLogger(_SPARKSPAWN_LOGGER_NAME)
    if normalized_name.startswith(f"{_SPARKSPAWN_LOGGER_NAME}."):
        return logging.getLogger(normalized_name)
    return logging.getLogger(f"{_SPARKSPAWN_LOGGER_NAME}.{normalized_name}")
