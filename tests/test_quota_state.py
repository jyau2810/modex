import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.quota import QuotaSnapshot, RateWindow, WatchState, evaluate_quota


class QuotaStateTests(unittest.TestCase):
    def test_notifies_once_when_limited_identity_recovers(self):
        state = WatchState()
        limited = QuotaSnapshot(
            identity="business-a",
            plan_type="business",
            primary=RateWindow(used_percent=100, resets_at=10, window_duration_mins=300),
            secondary=None,
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type="rate_limit_reached",
        )
        recovered = QuotaSnapshot(
            identity="business-a",
            plan_type="business",
            primary=RateWindow(used_percent=12, resets_at=20, window_duration_mins=300),
            secondary=None,
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type=None,
        )

        self.assertIsNone(evaluate_quota(state, limited))
        event = evaluate_quota(state, recovered)
        self.assertIsNotNone(event)
        self.assertEqual(event.identity, "business-a")
        self.assertEqual(event.kind, "recovered")
        self.assertIsNone(evaluate_quota(state, recovered))

    def test_waits_for_both_five_hour_and_weekly_windows_to_recover(self):
        state = WatchState()
        both_limited = QuotaSnapshot(
            identity="business-b",
            plan_type="business",
            primary=RateWindow(used_percent=100, resets_at=10, window_duration_mins=300),
            secondary=RateWindow(used_percent=100, resets_at=100, window_duration_mins=10080),
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type="rate_limit_reached",
        )
        primary_recovered = QuotaSnapshot(
            identity="business-b",
            plan_type="business",
            primary=RateWindow(used_percent=20, resets_at=20, window_duration_mins=300),
            secondary=RateWindow(used_percent=100, resets_at=100, window_duration_mins=10080),
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type="rate_limit_reached",
        )
        all_recovered = QuotaSnapshot(
            identity="business-b",
            plan_type="business",
            primary=RateWindow(used_percent=20, resets_at=20, window_duration_mins=300),
            secondary=RateWindow(used_percent=40, resets_at=120, window_duration_mins=10080),
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type=None,
        )

        self.assertIsNone(evaluate_quota(state, both_limited))
        self.assertIsNone(evaluate_quota(state, primary_recovered))
        self.assertEqual(evaluate_quota(state, all_recovered).kind, "recovered")

    def test_credits_available_counts_as_available(self):
        state = WatchState()
        limited_by_percent = QuotaSnapshot(
            identity="business-a",
            plan_type="business",
            primary=RateWindow(used_percent=100, resets_at=None, window_duration_mins=300),
            secondary=None,
            credits_has_credits=True,
            credits_unlimited=False,
            reached_type="rate_limit_reached",
        )

        self.assertFalse(limited_by_percent.is_limited)
        self.assertIsNone(evaluate_quota(state, limited_by_percent))


if __name__ == "__main__":
    unittest.main()
