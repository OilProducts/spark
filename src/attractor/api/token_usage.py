from __future__ import annotations

from dataclasses import dataclass, field
from decimal import Decimal, ROUND_HALF_UP
from typing import Any, Dict, Mapping, Optional


_USD_QUANTIZE = Decimal("0.000001")
_TOKENS_PER_MILLION = Decimal("1000000")
_DEFAULT_CURRENCY = "USD"


def _coerce_non_negative_int(value: Any) -> int:
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return max(0, value)
    if isinstance(value, float) and value.is_integer():
        return max(0, int(value))
    if isinstance(value, str):
        stripped = value.strip()
        if stripped.isdigit():
            return int(stripped)
    return 0


def _bucket_value(payload: Mapping[str, Any], *keys: str) -> int:
    for key in keys:
        if key in payload:
            return _coerce_non_negative_int(payload.get(key))
    return 0


@dataclass
class TokenUsageBucket:
    input_tokens: int = 0
    cached_input_tokens: int = 0
    output_tokens: int = 0
    total_tokens: int = 0

    def __post_init__(self) -> None:
        self.input_tokens = _coerce_non_negative_int(self.input_tokens)
        self.cached_input_tokens = min(
            self.input_tokens,
            _coerce_non_negative_int(self.cached_input_tokens),
        )
        self.output_tokens = _coerce_non_negative_int(self.output_tokens)
        normalized_total = _coerce_non_negative_int(self.total_tokens)
        baseline_total = self.input_tokens + self.output_tokens
        self.total_tokens = normalized_total if normalized_total > 0 else baseline_total
        self.total_tokens = max(self.total_tokens, baseline_total)

    def copy(self) -> "TokenUsageBucket":
        return TokenUsageBucket(
            input_tokens=self.input_tokens,
            cached_input_tokens=self.cached_input_tokens,
            output_tokens=self.output_tokens,
            total_tokens=self.total_tokens,
        )

    def add(self, other: "TokenUsageBucket") -> None:
        self.input_tokens += other.input_tokens
        self.cached_input_tokens += other.cached_input_tokens
        self.output_tokens += other.output_tokens
        self.total_tokens += other.total_tokens

    def delta_from(self, previous: "TokenUsageBucket") -> "TokenUsageBucket":
        return TokenUsageBucket(
            input_tokens=max(0, self.input_tokens - previous.input_tokens),
            cached_input_tokens=max(0, self.cached_input_tokens - previous.cached_input_tokens),
            output_tokens=max(0, self.output_tokens - previous.output_tokens),
            total_tokens=max(0, self.total_tokens - previous.total_tokens),
        )

    def has_any_usage(self) -> bool:
        return any(
            (
                self.input_tokens > 0,
                self.cached_input_tokens > 0,
                self.output_tokens > 0,
                self.total_tokens > 0,
            )
        )

    def to_dict(self) -> Dict[str, int]:
        return {
            "input_tokens": self.input_tokens,
            "cached_input_tokens": self.cached_input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any] | None) -> Optional["TokenUsageBucket"]:
        if not isinstance(payload, Mapping):
            return None
        bucket = cls(
            input_tokens=_bucket_value(payload, "input_tokens", "inputTokens"),
            cached_input_tokens=_bucket_value(payload, "cached_input_tokens", "cachedInputTokens"),
            output_tokens=_bucket_value(payload, "output_tokens", "outputTokens"),
            total_tokens=_bucket_value(payload, "total_tokens", "totalTokens"),
        )
        return bucket if bucket.has_any_usage() else None


@dataclass
class ModelEstimatedCost:
    currency: str = _DEFAULT_CURRENCY
    amount: Optional[float] = None
    status: str = "unpriced"

    def to_dict(self) -> Dict[str, object]:
        return {
            "currency": self.currency,
            "amount": self.amount,
            "status": self.status,
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any] | None) -> Optional["ModelEstimatedCost"]:
        if not isinstance(payload, Mapping):
            return None
        raw_amount = payload.get("amount")
        amount: Optional[float]
        if isinstance(raw_amount, (int, float)):
            amount = float(raw_amount)
        elif raw_amount is None:
            amount = None
        else:
            amount = None
        raw_status = payload.get("status")
        status = str(raw_status).strip() if raw_status is not None else "unpriced"
        raw_currency = payload.get("currency")
        currency = str(raw_currency).strip() if raw_currency is not None else _DEFAULT_CURRENCY
        return cls(currency=currency or _DEFAULT_CURRENCY, amount=amount, status=status or "unpriced")


@dataclass
class EstimatedModelCost:
    currency: str = _DEFAULT_CURRENCY
    amount: float = 0.0
    status: str = "unpriced"
    unpriced_models: list[str] = field(default_factory=list)
    by_model: dict[str, ModelEstimatedCost] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, object]:
        return {
            "currency": self.currency,
            "amount": self.amount,
            "status": self.status,
            "unpriced_models": list(self.unpriced_models),
            "by_model": {
                model_id: estimated_cost.to_dict()
                for model_id, estimated_cost in sorted(self.by_model.items())
            },
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any] | None) -> Optional["EstimatedModelCost"]:
        if not isinstance(payload, Mapping):
            return None
        by_model_payload = payload.get("by_model")
        by_model: dict[str, ModelEstimatedCost] = {}
        if isinstance(by_model_payload, Mapping):
            for model_id, model_cost_payload in by_model_payload.items():
                if not isinstance(model_id, str):
                    continue
                parsed_cost = ModelEstimatedCost.from_dict(model_cost_payload if isinstance(model_cost_payload, Mapping) else None)
                if parsed_cost is not None:
                    by_model[model_id] = parsed_cost
        raw_amount = payload.get("amount")
        amount = float(raw_amount) if isinstance(raw_amount, (int, float)) else 0.0
        raw_status = payload.get("status")
        status = str(raw_status).strip() if raw_status is not None else "unpriced"
        raw_currency = payload.get("currency")
        currency = str(raw_currency).strip() if raw_currency is not None else _DEFAULT_CURRENCY
        raw_unpriced_models = payload.get("unpriced_models")
        unpriced_models = (
            [str(model_id) for model_id in raw_unpriced_models if isinstance(model_id, str)]
            if isinstance(raw_unpriced_models, list)
            else []
        )
        return cls(
            currency=currency or _DEFAULT_CURRENCY,
            amount=amount,
            status=status or "unpriced",
            unpriced_models=unpriced_models,
            by_model=by_model,
        )


@dataclass
class TokenUsageBreakdown:
    input_tokens: int = 0
    cached_input_tokens: int = 0
    output_tokens: int = 0
    total_tokens: int = 0
    by_model: dict[str, TokenUsageBucket] = field(default_factory=dict)

    def __post_init__(self) -> None:
        aggregate = TokenUsageBucket(
            input_tokens=self.input_tokens,
            cached_input_tokens=self.cached_input_tokens,
            output_tokens=self.output_tokens,
            total_tokens=self.total_tokens,
        )
        self.input_tokens = aggregate.input_tokens
        self.cached_input_tokens = aggregate.cached_input_tokens
        self.output_tokens = aggregate.output_tokens
        self.total_tokens = aggregate.total_tokens

    def copy(self) -> "TokenUsageBreakdown":
        return TokenUsageBreakdown(
            input_tokens=self.input_tokens,
            cached_input_tokens=self.cached_input_tokens,
            output_tokens=self.output_tokens,
            total_tokens=self.total_tokens,
            by_model={model_id: usage.copy() for model_id, usage in self.by_model.items()},
        )

    def add_for_model(self, model_id: str, delta: TokenUsageBucket) -> None:
        normalized_model_id = str(model_id or "").strip() or "unknown"
        self.input_tokens += delta.input_tokens
        self.cached_input_tokens += delta.cached_input_tokens
        self.output_tokens += delta.output_tokens
        self.total_tokens += delta.total_tokens
        existing = self.by_model.get(normalized_model_id)
        if existing is None:
            existing = TokenUsageBucket()
            self.by_model[normalized_model_id] = existing
        existing.add(delta)

    def to_dict(self) -> Dict[str, object]:
        return {
            "input_tokens": self.input_tokens,
            "cached_input_tokens": self.cached_input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "by_model": {
                model_id: usage.to_dict()
                for model_id, usage in sorted(self.by_model.items())
            },
        }

    @classmethod
    def from_dict(cls, payload: Mapping[str, Any] | None) -> Optional["TokenUsageBreakdown"]:
        if not isinstance(payload, Mapping):
            return None
        by_model_payload = payload.get("by_model")
        by_model: dict[str, TokenUsageBucket] = {}
        if isinstance(by_model_payload, Mapping):
            for model_id, usage_payload in by_model_payload.items():
                if not isinstance(model_id, str):
                    continue
                parsed_usage = TokenUsageBucket.from_dict(usage_payload if isinstance(usage_payload, Mapping) else None)
                if parsed_usage is not None:
                    by_model[model_id] = parsed_usage
        breakdown = cls(
            input_tokens=_bucket_value(payload, "input_tokens", "inputTokens"),
            cached_input_tokens=_bucket_value(payload, "cached_input_tokens", "cachedInputTokens"),
            output_tokens=_bucket_value(payload, "output_tokens", "outputTokens"),
            total_tokens=_bucket_value(payload, "total_tokens", "totalTokens"),
            by_model=by_model,
        )
        return breakdown if breakdown.has_any_usage() else None

    def has_any_usage(self) -> bool:
        return any(
            (
                self.input_tokens > 0,
                self.cached_input_tokens > 0,
                self.output_tokens > 0,
                self.total_tokens > 0,
                bool(self.by_model),
            )
        )


def parse_app_server_token_usage_bucket(payload: Mapping[str, Any] | None) -> Optional[TokenUsageBucket]:
    return TokenUsageBucket.from_dict(payload)


def compute_live_usage_delta(
    token_usage_payload: Mapping[str, Any] | None,
    previous_total: TokenUsageBucket | None,
) -> tuple[TokenUsageBucket | None, TokenUsageBucket | None]:
    if not isinstance(token_usage_payload, Mapping):
        return None, previous_total.copy() if previous_total is not None else None

    current_total = parse_app_server_token_usage_bucket(
        token_usage_payload.get("total") if isinstance(token_usage_payload.get("total"), Mapping) else None
    )
    last_bucket = parse_app_server_token_usage_bucket(
        token_usage_payload.get("last") if isinstance(token_usage_payload.get("last"), Mapping) else None
    )

    if last_bucket is not None and last_bucket.has_any_usage():
        if current_total is None:
            current_total = previous_total.copy() if previous_total is not None else TokenUsageBucket()
            current_total.add(last_bucket)
        return last_bucket, current_total

    if current_total is None:
        return None, previous_total.copy() if previous_total is not None else None

    if previous_total is None:
        return current_total, current_total

    return current_total.delta_from(previous_total), current_total


@dataclass(frozen=True)
class ModelPricing:
    input_per_million: Decimal
    cached_input_per_million: Decimal
    output_per_million: Decimal


# Rates checked against official OpenAI model documentation and pricing pages on 2026-04-23.
MODEL_PRICING_CATALOG: dict[str, ModelPricing] = {
    "codex-mini-latest": ModelPricing(Decimal("1.50"), Decimal("0.375"), Decimal("6.00")),
    "gpt-4.1": ModelPricing(Decimal("2.00"), Decimal("0.50"), Decimal("8.00")),
    "gpt-4.1-mini": ModelPricing(Decimal("0.40"), Decimal("0.10"), Decimal("1.60")),
    "gpt-4.1-nano": ModelPricing(Decimal("0.10"), Decimal("0.025"), Decimal("0.40")),
    "gpt-5": ModelPricing(Decimal("1.25"), Decimal("0.125"), Decimal("10.00")),
    "gpt-5-codex": ModelPricing(Decimal("1.25"), Decimal("0.125"), Decimal("10.00")),
    "gpt-5-mini": ModelPricing(Decimal("0.25"), Decimal("0.025"), Decimal("2.00")),
    "gpt-5-nano": ModelPricing(Decimal("0.05"), Decimal("0.005"), Decimal("0.40")),
    "gpt-5.1": ModelPricing(Decimal("1.25"), Decimal("0.125"), Decimal("10.00")),
    "gpt-5.1-codex": ModelPricing(Decimal("1.25"), Decimal("0.125"), Decimal("10.00")),
    "gpt-5.2": ModelPricing(Decimal("1.75"), Decimal("0.175"), Decimal("14.00")),
    "gpt-5.2-codex": ModelPricing(Decimal("1.75"), Decimal("0.175"), Decimal("14.00")),
    "gpt-5.3-codex": ModelPricing(Decimal("1.75"), Decimal("0.175"), Decimal("14.00")),
    "gpt-5.5": ModelPricing(Decimal("5.00"), Decimal("0.50"), Decimal("30.00")),
    "gpt-5.4": ModelPricing(Decimal("2.50"), Decimal("0.25"), Decimal("15.00")),
    "gpt-5.4-mini": ModelPricing(Decimal("0.75"), Decimal("0.075"), Decimal("4.50")),
    "gpt-5.4-nano": ModelPricing(Decimal("0.20"), Decimal("0.02"), Decimal("1.25")),
}


def estimate_model_cost(breakdown: TokenUsageBreakdown | None) -> Optional[EstimatedModelCost]:
    if breakdown is None or not breakdown.has_any_usage():
        return None

    subtotal = Decimal("0")
    by_model_costs: dict[str, ModelEstimatedCost] = {}
    unpriced_models: list[str] = []

    for model_id, usage in sorted(breakdown.by_model.items()):
        pricing = MODEL_PRICING_CATALOG.get(model_id)
        if pricing is None:
            unpriced_models.append(model_id)
            by_model_costs[model_id] = ModelEstimatedCost(status="unpriced")
            continue

        uncached_input_tokens = max(0, usage.input_tokens - usage.cached_input_tokens)
        model_amount = (
            (Decimal(uncached_input_tokens) * pricing.input_per_million)
            + (Decimal(usage.cached_input_tokens) * pricing.cached_input_per_million)
            + (Decimal(usage.output_tokens) * pricing.output_per_million)
        ) / _TOKENS_PER_MILLION
        subtotal += model_amount
        by_model_costs[model_id] = ModelEstimatedCost(
            amount=float(model_amount.quantize(_USD_QUANTIZE, rounding=ROUND_HALF_UP)),
            status="estimated",
        )

    status = "estimated"
    if unpriced_models and by_model_costs and len(unpriced_models) != len(breakdown.by_model):
        status = "partial_unpriced"
    elif unpriced_models and len(unpriced_models) == len(breakdown.by_model):
        status = "unpriced"

    return EstimatedModelCost(
        amount=float(subtotal.quantize(_USD_QUANTIZE, rounding=ROUND_HALF_UP)),
        status=status,
        unpriced_models=unpriced_models,
        by_model=by_model_costs,
    )
