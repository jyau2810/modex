from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


LIMITED_REACHED_TYPES = {
    "rate_limit_reached",
    "workspace_owner_credits_depleted",
    "workspace_member_credits_depleted",
    "workspace_owner_usage_limit_reached",
    "workspace_member_usage_limit_reached",
}


@dataclass(frozen=True)
class RateWindow:
    used_percent: int
    resets_at: Optional[int]
    window_duration_mins: Optional[int]

    @property
    def is_limited(self) -> bool:
        return self.used_percent >= 100


@dataclass(frozen=True)
class QuotaSnapshot:
    identity: str
    plan_type: Optional[str]
    primary: Optional[RateWindow]
    secondary: Optional[RateWindow]
    credits_has_credits: Optional[bool]
    credits_unlimited: Optional[bool]
    reached_type: Optional[str]

    @property
    def has_usable_credits(self) -> bool:
        return self.credits_unlimited is True or self.credits_has_credits is True

    @property
    def is_limited(self) -> bool:
        if self.has_usable_credits:
            return False
        reached = self.reached_type in LIMITED_REACHED_TYPES
        window_limited = any(
            window.is_limited for window in (self.primary, self.secondary) if window is not None
        )
        return reached or window_limited

    @property
    def blocking_windows(self) -> list[RateWindow]:
        if self.has_usable_credits:
            return []
        return [
            window
            for window in (self.primary, self.secondary)
            if window is not None and window.is_limited
        ]


@dataclass(frozen=True)
class QuotaEvent:
    identity: str
    kind: str
    snapshot: QuotaSnapshot


@dataclass
class WatchState:
    limited_identities: set[str] = field(default_factory=set)


def evaluate_quota(state: WatchState, snapshot: QuotaSnapshot) -> Optional[QuotaEvent]:
    was_limited = snapshot.identity in state.limited_identities
    is_limited = snapshot.is_limited
    if is_limited:
        state.limited_identities.add(snapshot.identity)
        return None
    if was_limited:
        state.limited_identities.remove(snapshot.identity)
        return QuotaEvent(identity=snapshot.identity, kind="recovered", snapshot=snapshot)
    return None


def snapshot_from_rate_limits(identity: str, payload: dict[str, Any]) -> QuotaSnapshot:
    rate_limits = payload.get("rateLimits") or payload
    credits = rate_limits.get("credits") or {}
    return QuotaSnapshot(
        identity=identity,
        plan_type=rate_limits.get("planType"),
        primary=_window(rate_limits.get("primary")),
        secondary=_window(rate_limits.get("secondary")),
        credits_has_credits=credits.get("hasCredits"),
        credits_unlimited=credits.get("unlimited"),
        reached_type=rate_limits.get("rateLimitReachedType"),
    )


def _window(raw: object) -> Optional[RateWindow]:
    if not isinstance(raw, dict):
        return None
    return RateWindow(
        used_percent=int(raw.get("usedPercent", 0)),
        resets_at=raw.get("resetsAt"),
        window_duration_mins=raw.get("windowDurationMins"),
    )
