import sys
import base64
import json
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.app_config import AppIdentity, AppSettings
import codex_multi_account.gui_model as gui_model
from codex_multi_account.gui_model import (
    GuiActionResult,
    GuiViewModel,
    login_status,
    open_codex_app,
    read_quota_snapshot,
    run_login,
)
from codex_multi_account.quota import QuotaSnapshot, RateWindow


def json_dumps(value):
    return json.dumps(value)


def jwt_with_claims(claims):
    def encode(value):
        raw = json.dumps(value, separators=(",", ":")).encode()
        return base64.urlsafe_b64encode(raw).decode().rstrip("=")

    return f"{encode({'alg': 'none'})}.{encode(claims)}.signature"


def auth_json(email, sub, account_id="acct-team", plan="team"):
    return json_dumps(
        {
            "tokens": {
                "account_id": account_id,
                "id_token": jwt_with_claims(
                    {
                        "email": email,
                        "sub": sub,
                        "https://api.openai.com/auth": {
                            "chatgpt_plan_type": plan,
                        },
                    }
                ),
            }
        }
    )


class GuiViewModelTests(unittest.TestCase):
    def make_settings(self):
        return AppSettings(
            codex_binary="codex",
            app_name="Codex",
            poll_seconds=60,
            source_home=Path("/Users/alex/.codex"),
            has_completed_setup=True,
            identities=[
                AppIdentity("Enterprise", Path("/Users/alex/.codex-enterprise"), False, None),
                AppIdentity("Business A", Path("/Users/alex/.codex-business-a"), True, "workspace-a"),
            ],
        )

    def make_empty_settings(self):
        return AppSettings(
            codex_binary="codex",
            app_name="Codex",
            poll_seconds=60,
            source_home=Path("/Users/alex/.codex"),
            has_completed_setup=False,
            identities=[],
        )

    def test_selects_first_identity_by_default(self):
        model = GuiViewModel(self.make_settings())

        self.assertEqual(model.selected_identity().name, "Enterprise")

    def test_refresh_quota_stores_snapshot_and_clears_previous_error(self):
        calls = []

        def quota_reader(settings, identity):
            calls.append((settings.codex_binary, identity.codex_home))
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="business",
                primary=RateWindow(used_percent=42, resets_at=123, window_duration_mins=300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(self.make_settings(), quota_reader=quota_reader)
        model.errors["Business A"] = "old error"

        result = model.refresh_identity("Business A")

        self.assertTrue(result.ok)
        self.assertEqual(calls, [("codex", Path("/Users/alex/.codex-business-a"))])
        self.assertEqual(model.snapshots["Business A"].primary.used_percent, 42)
        self.assertNotIn("Business A", model.errors)

    def test_refresh_quota_does_not_notify_without_recovery_transition(self):
        notifications = []

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="team",
                primary=RateWindow(used_percent=42, resets_at=None, window_duration_mins=300),
                secondary=RateWindow(used_percent=88, resets_at=None, window_duration_mins=10080),
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(
            self.make_settings(),
            quota_reader=quota_reader,
            notifier=lambda title, message: notifications.append((title, message)),
        )

        result = model.refresh_identity("Business A")

        self.assertTrue(result.ok)
        self.assertEqual(notifications, [])

    def test_refresh_quota_records_error_without_crashing(self):
        def quota_reader(settings, identity):
            raise RuntimeError("not logged in")

        model = GuiViewModel(self.make_settings(), quota_reader=quota_reader)

        result = model.refresh_identity("Business A")

        self.assertFalse(result.ok)
        self.assertIn("not logged in", result.message)
        self.assertEqual(model.errors["Business A"], "not logged in")

    def test_toggle_monitor_marks_settings_dirty(self):
        model = GuiViewModel(self.make_settings())

        model.set_monitor("Enterprise", True)

        self.assertTrue(model.settings.identities[0].monitor)
        self.assertTrue(model.is_dirty)

    def test_set_business_plan_marks_identity_for_monitoring(self):
        model = GuiViewModel(self.make_settings())

        model.set_business_plan("Enterprise", True)

        self.assertTrue(model.settings.identities[0].monitor)
        self.assertTrue(model.is_dirty)

    def test_refresh_business_snapshot_keeps_enterprise_identity_unmonitored(self):
        settings = self.make_settings()
        settings.identities[0].monitor = False

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="business",
                primary=RateWindow(used_percent=12, resets_at=None, window_duration_mins=300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(settings, quota_reader=quota_reader)

        model.refresh_identity("Enterprise")

        self.assertFalse(settings.identities[0].monitor)
        self.assertEqual(model.quota_display("Enterprise").plan, "企业版")

    def test_refresh_team_snapshot_marks_identity_as_business_plan(self):
        settings = self.make_settings()
        settings.identities[0].monitor = False

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="team",
                primary=RateWindow(used_percent=12, resets_at=None, window_duration_mins=300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(settings, quota_reader=quota_reader)

        model.refresh_identity("Enterprise")

        self.assertTrue(settings.identities[0].monitor)
        self.assertEqual(model.quota_display("Enterprise").plan, "团队版")

    def test_refresh_non_team_snapshot_disables_monitoring(self):
        settings = self.make_settings()
        settings.identities[1].monitor = True

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="pro",
                primary=RateWindow(used_percent=12, resets_at=None, window_duration_mins=300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(settings, quota_reader=quota_reader)

        model.refresh_identity("Business A")

        self.assertFalse(settings.identities[1].monitor)

    def test_detects_current_identity_from_running_codex_home(self):
        settings = self.make_settings()

        model = GuiViewModel(
            settings,
            current_home_reader=lambda _settings: Path("/Users/alex/.codex-business-a"),
        )

        self.assertEqual(model.current_identity_name, "Business A")

    def test_falls_back_to_persisted_current_identity(self):
        settings = self.make_settings()
        settings.current_identity_name = "Business A"

        model = GuiViewModel(settings, current_home_reader=lambda _settings: None)

        self.assertEqual(model.current_identity_name, "Business A")

    def test_unmatched_running_source_home_does_not_use_persisted_current_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            business = root / "business"
            source.mkdir()
            business.mkdir()
            (source / "auth.json").write_text('{"tokens": {"account_id": "acct-enterprise"}}')
            (business / "auth.json").write_text('{"tokens": {"account_id": "acct-business"}}')
            settings = AppSettings(
                source_home=source,
                current_identity_name="商业版",
                identities=[AppIdentity("商业版", business, False)],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: source)

            self.assertIsNone(model.current_identity_name)
            self.assertIsNone(settings.current_identity_name)
            self.assertTrue(model.is_dirty)

    def test_source_home_maps_to_identity_with_same_auth_account(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            enterprise = root / "enterprise"
            business = root / "business"
            source.mkdir()
            enterprise.mkdir()
            business.mkdir()
            (source / "auth.json").write_text('{"tokens": {"account_id": "acct-enterprise"}}')
            (enterprise / "auth.json").write_text('{"tokens": {"account_id": "acct-enterprise"}}')
            (business / "auth.json").write_text('{"tokens": {"account_id": "acct-business"}}')
            settings = AppSettings(
                source_home=source,
                identities=[
                    AppIdentity("企业版", enterprise, False),
                    AppIdentity("商业版", business, False),
                ],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: source)

            self.assertEqual(model.current_identity_name, "企业版")

    def test_shared_team_account_id_does_not_match_different_user(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            yaoji = root / "yaoji-team"
            source.mkdir()
            yaoji.mkdir()
            (source / "auth.json").write_text(auth_json("yaoji+1@example.com", "user-yaoji-plus-one"))
            (yaoji / "auth.json").write_text(auth_json("yaoji@example.com", "user-yaoji"))
            settings = AppSettings(
                source_home=source,
                current_identity_name="yaoji@example.com · 团队版",
                identities=[AppIdentity("yaoji@example.com · 团队版", yaoji, False)],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: source)

            self.assertIsNone(model.current_identity_name)
            self.assertIsNone(settings.current_identity_name)

    def test_shared_team_account_id_matches_identity_by_user_claims(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            yaoji = root / "yaoji-team"
            yaoji_plus_one = root / "yaoji-plus-one-team"
            source.mkdir()
            yaoji.mkdir()
            yaoji_plus_one.mkdir()
            team_account = "acct-shared-team"
            (source / "auth.json").write_text(
                auth_json("yaoji+1@example.com", "user-yaoji-plus-one", team_account)
            )
            (yaoji / "auth.json").write_text(auth_json("yaoji@example.com", "user-yaoji", team_account))
            (yaoji_plus_one / "auth.json").write_text(
                auth_json("yaoji+1@example.com", "user-yaoji-plus-one", team_account)
            )
            settings = AppSettings(
                source_home=source,
                current_identity_name="yaoji@example.com · 团队版",
                identities=[
                    AppIdentity("yaoji@example.com · 团队版", yaoji, False),
                    AppIdentity("yaoji+1@example.com · 团队版", yaoji_plus_one, False),
                ],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: source)

            self.assertEqual(model.current_identity_name, "yaoji+1@example.com · 团队版")
            self.assertEqual(settings.current_identity_name, "yaoji+1@example.com · 团队版")

    def test_source_home_keeps_persisted_current_when_auth_matches_duplicate_accounts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            business_a = root / "business-a"
            business_b = root / "business-b"
            source.mkdir()
            business_a.mkdir()
            business_b.mkdir()
            auth = '{"tokens": {"account_id": "acct-business"}}'
            (source / "auth.json").write_text(auth)
            (business_a / "auth.json").write_text(auth)
            (business_b / "auth.json").write_text(auth)
            settings = AppSettings(
                source_home=source,
                current_identity_name="商业版 B",
                identities=[
                    AppIdentity("商业版 A", business_a, False),
                    AppIdentity("商业版 B", business_b, False),
                ],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: source)

            self.assertEqual(model.current_identity_name, "商业版 B")
            self.assertEqual(settings.current_identity_name, "商业版 B")

    def test_switch_identity_activates_codex_when_already_current(self):
        calls = []
        settings = self.make_settings()
        settings.current_identity_name = "Business A"

        def opener(settings, identity):
            calls.append(("switch", identity.name))
            return GuiActionResult(True, "switched")

        def activator(settings):
            calls.append(("activate", settings.app_name))
            return GuiActionResult(True, "activated")

        model = GuiViewModel(
            settings,
            opener=opener,
            activator=activator,
            current_home_reader=lambda _settings: None,
        )

        result = model.switch_identity("Business A")

        self.assertTrue(result.ok)
        self.assertEqual(calls, [("activate", "Codex")])

    def test_switch_identity_updates_current_after_successful_switch(self):
        calls = []
        settings = self.make_settings()
        settings.current_identity_name = "Enterprise"

        def opener(settings, identity):
            calls.append(identity.name)
            return GuiActionResult(True, "switched")

        model = GuiViewModel(
            settings,
            opener=opener,
            current_home_reader=lambda _settings: None,
        )

        result = model.switch_identity("Business A")

        self.assertTrue(result.ok)
        self.assertEqual(calls, ["Business A"])
        self.assertEqual(model.current_identity_name, "Business A")
        self.assertEqual(settings.current_identity_name, "Business A")

    def test_open_selected_identity_invokes_runner(self):
        calls = []

        def opener(settings, identity):
            calls.append(identity.name)
            return GuiActionResult(True, "opened")

        model = GuiViewModel(self.make_settings(), opener=opener)
        model.selected_identity_name = "Business A"

        result = model.open_selected_identity()

        self.assertTrue(result.ok)
        self.assertEqual(calls, ["Business A"])

    def test_quota_display_uses_clear_chinese_fields(self):
        model = GuiViewModel(self.make_settings())
        model.snapshots["Business A"] = QuotaSnapshot(
            identity="Business A",
            plan_type="team",
            primary=RateWindow(used_percent=42, resets_at=None, window_duration_mins=300),
            secondary=RateWindow(used_percent=88, resets_at=None, window_duration_mins=10080),
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type=None,
        )

        display = model.quota_display("Business A")

        self.assertEqual(display.status, "可用")
        self.assertEqual(display.plan, "团队版")
        self.assertEqual(display.primary_label, "5小时已用 42%")
        self.assertEqual(display.primary_percent, 42)
        self.assertEqual(display.secondary_label, "每周已用 88%")
        self.assertEqual(display.secondary_percent, 88)

    def test_quota_display_includes_reset_time(self):
        model = GuiViewModel(self.make_settings())
        resets_at = 1_234_567_890
        model.snapshots["Business A"] = QuotaSnapshot(
            identity="Business A",
            plan_type="team",
            primary=RateWindow(42, resets_at, 300),
            secondary=None,
            credits_has_credits=False,
            credits_unlimited=False,
            reached_type=None,
        )

        display = model.quota_display("Business A")

        reset_label = time.strftime("%m-%d %H:%M", time.localtime(resets_at))
        self.assertEqual(display.primary_label, f"5小时已用 42% · {reset_label}刷新")

    def test_business_plan_type_displays_enterprise_without_name_hint(self):
        model = GuiViewModel(self.make_settings())
        model.snapshots["Business A"] = QuotaSnapshot(
            identity="Business A",
            plan_type="business",
            primary=None,
            secondary=None,
            credits_has_credits=True,
            credits_unlimited=False,
            reached_type=None,
        )

        display = model.quota_display("Business A")

        self.assertEqual(display.plan, "企业版")

    def test_enterprise_name_without_plan_remains_unknown(self):
        model = GuiViewModel(self.make_settings())
        model.snapshots["Enterprise"] = QuotaSnapshot(
            identity="Enterprise",
            plan_type=None,
            primary=None,
            secondary=None,
            credits_has_credits=None,
            credits_unlimited=None,
            reached_type=None,
        )

        display = model.quota_display("Enterprise")

        self.assertEqual(display.plan, "计划未知")

    def test_quota_error_uses_auth_plan_type_as_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity_home = root / "identity"
            identity_home.mkdir()
            (identity_home / "auth.json").write_text(
                json_dumps(
                    {
                        "tokens": {
                            "access_token": jwt_with_claims(
                                {
                                    "https://api.openai.com/auth": {
                                        "chatgpt_plan_type": "business",
                                    },
                                }
                            )
                        }
                    }
                )
            )
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", identity_home, False)],
            )
            model = GuiViewModel(settings)
            model.errors["账号"] = "rate limit failed"

            display = model.quota_display("账号")

            self.assertEqual(display.plan, "企业版")

    def test_extracts_current_home_from_lsof_output(self):
        settings = self.make_settings()
        output = """
codex 123 yaoji 10u REG 1,18 100 /Users/alex/.codex-business-a/state_5.sqlite
codex 123 yaoji 11u REG 1,18 100 /Users/alex/.codex-business-a/logs_2.sqlite
"""

        current_home = gui_model._home_from_lsof_output(settings, output)

        self.assertEqual(current_home, Path("/Users/alex/.codex-business-a"))

    def test_extracts_source_home_from_lsof_output(self):
        settings = self.make_settings()
        output = """
codex 123 yaoji 10u REG 1,18 100 /Users/alex/.codex/state_5.sqlite
codex 123 yaoji 11u REG 1,18 100 /Users/alex/.codex/logs_2.sqlite
"""

        current_home = gui_model._home_from_lsof_output(settings, output)

        self.assertEqual(current_home, Path("/Users/alex/.codex"))

    def test_codex_process_home_parser_ignores_tooling_processes(self):
        settings = self.make_settings()
        line = (
            "/Applications/Codex.app/Contents/Resources/node_repl "
            "CODEX_HOME=/Users/alex/.codex-business-a"
        )

        self.assertIsNone(gui_model._codex_home_from_process_line(settings, line))

    def test_codex_process_home_parser_reads_desktop_app_processes(self):
        settings = self.make_settings()
        line = (
            "/Applications/Codex.app/Contents/MacOS/Codex "
            "CODEX_HOME=/Users/alex/.codex-business-a"
        )

        self.assertEqual(
            gui_model._codex_home_from_process_line(settings, line),
            Path("/Users/alex/.codex-business-a"),
        )

    def test_codex_process_home_parser_ignores_temporary_stdio_app_servers(self):
        settings = self.make_settings()
        line = (
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio:// "
            "CODEX_HOME=/Users/alex/.codex-business-a"
        )

        self.assertIsNone(gui_model._codex_home_from_process_line(settings, line))

    def test_codex_app_server_pids_ignore_temporary_stdio_servers(self):
        output = """
123 /Applications/Codex.app/Contents/Resources/codex app-server --listen stdio:// CODEX_HOME=/tmp/a
456 /Applications/Codex.app/Contents/Resources/codex app-server --analytics-default-enabled
"""

        self.assertEqual(gui_model._codex_app_server_pids(output), ["456"])

    def test_add_identity_selects_it_and_marks_setup_complete(self):
        settings = self.make_empty_settings()
        model = GuiViewModel(settings)
        identity = AppIdentity("商业版", Path("/Users/alex/.codex-business"), True)

        model.add_identity(identity)

        self.assertEqual(settings.identities, [identity])
        self.assertEqual(model.selected_identity_name, "商业版")
        self.assertTrue(settings.has_completed_setup)

    def test_add_identity_uses_auth_email_and_plan_as_name(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity_home = root / "identity"
            identity_home.mkdir()
            (identity_home / "auth.json").write_text(
                json_dumps(
                    {
                        "tokens": {
                            "id_token": jwt_with_claims(
                                {
                                    "email": "yaoji+1@example.com",
                                    "name": "Yaoji",
                                    "https://api.openai.com/auth": {
                                        "chatgpt_plan_type": "team",
                                    },
                                }
                            ),
                        }
                    }
                )
            )
            settings = self.make_empty_settings()
            model = GuiViewModel(settings)

            model.add_identity(AppIdentity("登录中", identity_home, True))

            self.assertEqual(settings.identities[0].name, "yaoji+1@example.com · 团队版")
            self.assertEqual(model.selected_identity_name, "yaoji+1@example.com · 团队版")

    def test_existing_identity_names_refresh_from_auth_without_custom_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity_home = root / "identity"
            identity_home.mkdir()
            (identity_home / "auth.json").write_text(
                json_dumps(
                    {
                        "tokens": {
                            "id_token": jwt_with_claims(
                                {
                                    "email": "yaoji@example.com",
                                    "https://api.openai.com/auth": {
                                        "chatgpt_plan_type": "business",
                                    },
                                }
                            ),
                        }
                    }
                )
            )
            settings = AppSettings(
                source_home=root / "source",
                current_identity_name="手动名称",
                identities=[AppIdentity("手动名称", identity_home, False)],
            )

            model = GuiViewModel(settings, current_home_reader=lambda _settings: None)

            self.assertEqual(settings.identities[0].name, "yaoji@example.com · 企业版")
            self.assertEqual(settings.current_identity_name, "yaoji@example.com · 企业版")
            self.assertEqual(model.current_identity_name, "yaoji@example.com · 企业版")

    def test_update_identity_keeps_selection_and_replaces_values(self):
        settings = self.make_settings()
        model = GuiViewModel(settings)
        model.selected_identity_name = "Business A"

        model.update_identity(
            "Business A",
            AppIdentity("商业版 A", Path("/Users/alex/.codex-business-new"), False),
        )

        self.assertEqual(settings.identities[1].name, "商业版 A")
        self.assertEqual(settings.identities[1].codex_home, Path("/Users/alex/.codex-business-new"))
        self.assertFalse(settings.identities[1].monitor)
        self.assertEqual(model.selected_identity_name, "商业版 A")

    def test_update_identity_name_preserves_home_and_monitor(self):
        settings = self.make_settings()
        model = GuiViewModel(settings)
        model.selected_identity_name = "Business A"

        model.update_identity_name("Business A", "商业版 A")

        self.assertEqual(settings.identities[1].name, "商业版 A")
        self.assertEqual(settings.identities[1].codex_home, Path("/Users/alex/.codex-business-a"))
        self.assertTrue(settings.identities[1].monitor)
        self.assertEqual(settings.identities[1].workspace_id, "workspace-a")
        self.assertEqual(model.selected_identity_name, "商业版 A")

    def test_update_identity_home_expands_user_and_preserves_name_and_monitor(self):
        settings = self.make_settings()
        model = GuiViewModel(settings)

        model.update_identity_home("Business A", "~/new-codex-home")

        self.assertEqual(settings.identities[1].name, "Business A")
        self.assertEqual(settings.identities[1].codex_home, Path("~/new-codex-home").expanduser())
        self.assertTrue(settings.identities[1].monitor)
        self.assertEqual(settings.identities[1].workspace_id, "workspace-a")

    def test_update_identity_home_renames_existing_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old_home = root / "old-home"
            new_home = root / "new-home"
            old_home.mkdir()
            (old_home / "auth.json").write_text("{}")
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", old_home, True, "workspace-a")],
            )
            model = GuiViewModel(settings, current_home_reader=lambda _settings: None)

            model.update_identity_home("账号", str(new_home))

            self.assertFalse(old_home.exists())
            self.assertTrue(new_home.exists())
            self.assertEqual((new_home / "auth.json").read_text(), "{}")
            self.assertEqual(settings.identities[0].codex_home, new_home)
            self.assertTrue(settings.identities[0].monitor)
            self.assertEqual(settings.identities[0].workspace_id, "workspace-a")

    def test_update_identity_home_rejects_existing_target_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old_home = root / "old-home"
            new_home = root / "new-home"
            old_home.mkdir()
            new_home.mkdir()
            (new_home / "marker").write_text("keep")
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", old_home, False)],
            )
            model = GuiViewModel(settings, current_home_reader=lambda _settings: None)

            with self.assertRaisesRegex(ValueError, "目标账号目录已存在"):
                model.update_identity_home("账号", str(new_home))

            self.assertTrue(old_home.exists())
            self.assertEqual((new_home / "marker").read_text(), "keep")
            self.assertEqual(settings.identities[0].codex_home, old_home)

    def test_delete_identity_updates_selection(self):
        settings = self.make_settings()
        model = GuiViewModel(settings)
        model.selected_identity_name = "Enterprise"

        model.delete_identity("Enterprise")

        self.assertEqual([identity.name for identity in settings.identities], ["Business A"])
        self.assertEqual(model.selected_identity_name, "Business A")

    def test_check_login_status_uses_runner(self):
        calls = []

        def status_reader(settings, identity):
            calls.append(identity.name)
            return identity.name == "Business A"

        model = GuiViewModel(self.make_settings(), login_status_reader=status_reader)

        self.assertTrue(model.login_completed(AppIdentity("Business A", Path("/tmp/a"))))
        self.assertFalse(model.login_completed(AppIdentity("Enterprise", Path("/tmp/e"))))
        self.assertEqual(calls, ["Business A", "Enterprise"])

    def test_local_auth_file_marks_identity_logged_in(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", root / "identity", False)],
            )
            (root / "identity").mkdir()
            (root / "identity" / "auth.json").write_text("{}")
            model = GuiViewModel(settings)

            self.assertTrue(model.is_logged_in_identity("账号"))

    def test_refresh_login_statuses_caches_successful_statuses(self):
        settings = self.make_settings()

        def status_reader(settings, identity):
            return identity.name == "Business A"

        model = GuiViewModel(settings, login_status_reader=status_reader)

        result = model.refresh_login_statuses()

        self.assertTrue(result.ok)
        self.assertFalse(model.is_logged_in_identity("Enterprise"))
        self.assertTrue(model.is_logged_in_identity("Business A"))

    def test_refresh_login_statuses_marks_rejected_local_auth_as_expired(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", root / "identity", False)],
            )
            (root / "identity").mkdir()
            (root / "identity" / "auth.json").write_text("{}")
            model = GuiViewModel(settings, login_status_reader=lambda _settings, _identity: False)

            result = model.refresh_login_statuses()

            self.assertTrue(result.ok)
            self.assertFalse(model.is_logged_in_identity("账号"))
            self.assertTrue(model.is_login_expired_identity("账号"))

    def test_successful_login_status_clears_expired_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", root / "identity", False)],
            )
            (root / "identity").mkdir()
            (root / "identity" / "auth.json").write_text("{}")
            completed = False

            def status_reader(settings, identity):
                return completed

            model = GuiViewModel(settings, login_status_reader=status_reader)
            model.refresh_login_statuses()

            completed = True
            model.refresh_login_statuses()

            self.assertTrue(model.is_logged_in_identity("账号"))
            self.assertFalse(model.is_login_expired_identity("账号"))

    def test_successful_quota_refresh_marks_identity_logged_in(self):
        settings = self.make_settings()

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="pro",
                primary=RateWindow(used_percent=12, resets_at=None, window_duration_mins=300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            )

        model = GuiViewModel(settings, quota_reader=quota_reader)

        model.refresh_identity("Enterprise")

        self.assertTrue(model.is_logged_in_identity("Enterprise"))

    def test_auth_error_during_quota_refresh_marks_identity_expired(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", root / "identity", False)],
            )
            (root / "identity").mkdir()
            (root / "identity" / "auth.json").write_text("{}")

            def quota_reader(settings, identity):
                raise RuntimeError("not logged in")

            model = GuiViewModel(settings, quota_reader=quota_reader)

            result = model.refresh_identity("账号")

            self.assertFalse(result.ok)
            self.assertTrue(model.is_login_expired_identity("账号"))
            self.assertFalse(model.is_logged_in_identity("账号"))

    def test_run_login_starts_codex_directly_and_returns_process(self):
        calls = []

        class FakeProcess:
            pid = 1234

        def fake_popen(args, **kwargs):
            calls.append((args, kwargs))
            return FakeProcess()

        original_popen = gui_model.subprocess.Popen
        gui_model.subprocess.Popen = fake_popen
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source_home = root / "source"
                source_home.mkdir()
                settings = AppSettings(
                    codex_binary="/opt/codex",
                    source_home=source_home,
                    identities=[],
                )
                identity = AppIdentity("账号", root / "identity", True)

                result = run_login(settings, identity)
        finally:
            gui_model.subprocess.Popen = original_popen

        self.assertTrue(result.ok)
        self.assertIsInstance(result.process, FakeProcess)
        self.assertEqual(calls[0][0], ["/opt/codex", "login", "-c", 'forced_login_method="chatgpt"'])
        self.assertEqual(calls[0][1]["env"]["CODEX_HOME"], str(identity.codex_home))
        self.assertIn("stdout", calls[0][1])
        self.assertIn("stderr", calls[0][1])
        self.assertFalse((identity.codex_home / "config.toml").exists())
        self.assertFalse((identity.codex_home / "log").exists())

    def test_login_cleanup_removes_runtime_files_for_managed_identity_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity_home = root / ".modex" / "123456"
            identity_home.mkdir(parents=True)
            (identity_home / "auth.json").write_text("{}")
            (identity_home / "config.toml").write_text("model = 'old'\n")
            (identity_home / "state_5.sqlite").write_text("runtime")
            settings = AppSettings(
                source_home=root / "source",
                identities=[AppIdentity("账号", identity_home, False)],
            )
            model = GuiViewModel(settings, current_home_reader=lambda _settings: None)

            model.cleanup_identity_home(settings.identities[0])

            self.assertEqual([path.name for path in identity_home.iterdir()], ["auth.json"])

    def test_quota_reader_uses_temporary_auth_home_without_polluting_identity_home(self):
        calls = []
        original_request = gui_model.request_rate_limits
        gui_model.request_rate_limits = lambda binary, home: calls.append((binary, home, (home / "auth.json").read_text())) or {
            "plan_type": "team",
            "primary": None,
            "secondary": None,
            "credits": {},
        }
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                identity_home = root / ".modex" / "123456"
                identity_home.mkdir(parents=True)
                (identity_home / "auth.json").write_text('{"tokens": {}}')
                settings = AppSettings(codex_binary="codex", source_home=root / "source")
                identity = AppIdentity("账号", identity_home)

                read_quota_snapshot(settings, identity)
                remaining = [path.name for path in identity_home.iterdir()]
        finally:
            gui_model.request_rate_limits = original_request

        self.assertEqual(len(calls), 1)
        self.assertNotEqual(calls[0][1], identity_home)
        self.assertEqual(calls[0][2], '{"tokens": {}}')
        self.assertEqual(remaining, ["auth.json"])

    def test_login_status_uses_temporary_auth_home_when_auth_exists(self):
        calls = []

        def fake_run(args, **kwargs):
            calls.append(kwargs["env"]["CODEX_HOME"])
            return gui_model.subprocess.CompletedProcess(args, 0, stdout="", stderr="")

        original_run = gui_model.subprocess.run
        original_resolver = gui_model.resolve_codex_binary
        gui_model.subprocess.run = fake_run
        gui_model.resolve_codex_binary = lambda configured: configured
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                identity_home = root / ".modex" / "123456"
                identity_home.mkdir(parents=True)
                (identity_home / "auth.json").write_text("{}")
                (identity_home / "state_5.sqlite").write_text("runtime")
                settings = AppSettings(codex_binary="codex", source_home=root / "source")
                identity = AppIdentity("账号", identity_home)

                self.assertTrue(login_status(settings, identity))
                remaining = sorted(path.name for path in identity_home.iterdir())
        finally:
            gui_model.subprocess.run = original_run
            gui_model.resolve_codex_binary = original_resolver

        self.assertEqual(len(calls), 1)
        self.assertNotEqual(Path(calls[0]), identity_home)
        self.assertEqual(remaining, ["auth.json", "state_5.sqlite"])

    def test_open_codex_app_switches_auth_and_starts_with_source_home(self):
        calls = []

        class FakeProcess:
            pid = 1234

        def fake_run(args, **kwargs):
            calls.append(("run", args, kwargs))

        def fake_popen(args, **kwargs):
            calls.append(("popen", args, kwargs))
            return FakeProcess()

        original_run = gui_model.subprocess.run
        original_popen = gui_model.subprocess.Popen
        original_resolver = gui_model.resolve_codex_binary
        original_platform_system = gui_model.platform.system
        gui_model.subprocess.run = fake_run
        gui_model.subprocess.Popen = fake_popen
        gui_model.resolve_codex_binary = lambda configured: "/Applications/Codex.app/Contents/Resources/codex"
        gui_model.platform.system = lambda: "Darwin"
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source_home = root / "source"
                source_home.mkdir()
                (source_home / "auth.json").write_text('{"account_id": "old"}')
                (source_home / "config.toml").write_text('[projects."/tmp/keep"]\ntrust_level = "trusted"\n')
                sessions = source_home / "sessions"
                sessions.mkdir()
                (sessions / "conversation.jsonl").write_text("{}\n")
                settings = AppSettings(
                    codex_binary="codex",
                    app_name='Codex "Preview"',
                    source_home=source_home,
                    identities=[],
                )
                identity = AppIdentity("账号", root / "identity", True)
                identity.codex_home.mkdir()
                (identity.codex_home / "auth.json").write_text('{"account_id": "new"}')

                result = open_codex_app(settings, identity)
                source_auth = (settings.source_home / "auth.json").read_text()
                source_config = (settings.source_home / "config.toml").read_text()
                source_session = (settings.source_home / "sessions" / "conversation.jsonl").read_text()
        finally:
            gui_model.subprocess.run = original_run
            gui_model.subprocess.Popen = original_popen
            gui_model.resolve_codex_binary = original_resolver
            gui_model.platform.system = original_platform_system

        self.assertTrue(result.ok)
        self.assertEqual(calls[0][0], "run")
        self.assertEqual(calls[0][1][0:2], ["osascript", "-e"])
        self.assertIn('tell application "Codex \\"Preview\\""', calls[0][1][2])
        self.assertEqual(calls[1][0], "popen")
        self.assertEqual(
            calls[1][1],
            ["/Applications/Codex.app/Contents/Resources/codex", "app"],
        )
        self.assertEqual(calls[1][2]["env"]["CODEX_HOME"], str(settings.source_home))
        self.assertEqual(source_auth, '{"account_id": "new"}')
        self.assertIn('[projects."/tmp/keep"]', source_config)
        self.assertEqual(source_session, "{}\n")

    def test_open_codex_app_removes_root_project_and_uses_valid_launch_cwd(self):
        calls = []

        class FakeProcess:
            pid = 1234

        def fake_popen(args, **kwargs):
            calls.append((args, kwargs))
            return FakeProcess()

        original_popen = gui_model.subprocess.Popen
        original_resolver = gui_model.resolve_codex_binary
        original_platform_system = gui_model.platform.system
        gui_model.subprocess.Popen = fake_popen
        gui_model.resolve_codex_binary = lambda configured: "/Applications/Codex.app/Contents/Resources/codex"
        gui_model.platform.system = lambda: "Linux"
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source_home = root / "source"
                project = root / "project"
                source_home.mkdir()
                project.mkdir()
                (source_home / "auth.json").write_text('{"account_id": "old"}')
                (source_home / ".codex-global-state.json").write_text(
                    json_dumps(
                        {
                            "project-order": ["/", str(project)],
                            "electron-saved-workspace-roots": ["/", str(project)],
                            "active-workspace-roots": ["/"],
                        }
                    )
                )
                settings = AppSettings(
                    codex_binary="codex",
                    source_home=source_home,
                    identities=[],
                )
                identity = AppIdentity("账号", root / "identity", True)
                identity.codex_home.mkdir()
                (identity.codex_home / "auth.json").write_text('{"account_id": "new"}')

                result = open_codex_app(settings, identity)
                state = json.loads((source_home / ".codex-global-state.json").read_text())
        finally:
            gui_model.subprocess.Popen = original_popen
            gui_model.resolve_codex_binary = original_resolver
            gui_model.platform.system = original_platform_system

        self.assertTrue(result.ok)
        self.assertEqual(calls[0][1]["env"]["CODEX_HOME"], str(settings.source_home))
        self.assertEqual(calls[0][1]["cwd"], str(project))
        self.assertEqual(state["project-order"], [str(project)])
        self.assertEqual(state["electron-saved-workspace-roots"], [str(project)])
        self.assertEqual(state["active-workspace-roots"], [str(project)])

    def test_monitor_tick_notifies_recovery_once(self):
        snapshots = [
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(100, 10, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type="rate_limit_reached",
            ),
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(20, 20, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            ),
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(18, 30, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            ),
        ]
        notifications = []

        def quota_reader(settings, identity):
            return snapshots.pop(0)

        model = GuiViewModel(
            self.make_settings(),
            quota_reader=quota_reader,
            notifier=lambda title, message: notifications.append((title, message)),
        )

        model.monitor_tick()
        model.monitor_tick()
        model.monitor_tick()

        self.assertEqual(len(notifications), 1)
        title, message = notifications[0]
        self.assertEqual(title, "Codex 配额已恢复")
        self.assertIn("Business A", message)
        self.assertIn("可用", message)

    def test_pending_notification_list_is_separate_from_monitor_list(self):
        snapshots = [
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(20, 10, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            ),
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(100, 20, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type="rate_limit_reached",
            ),
            QuotaSnapshot(
                identity="Business A",
                plan_type="team",
                primary=RateWindow(30, 30, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type=None,
            ),
        ]
        notifications = []

        def quota_reader(settings, identity):
            return snapshots.pop(0)

        model = GuiViewModel(
            self.make_settings(),
            quota_reader=quota_reader,
            notifier=lambda title, message: notifications.append((title, message)),
        )
        model.refresh_identity("Business A")

        self.assertTrue(model.identity_named("Business A").monitor)
        self.assertEqual(model.watch_state.limited_identities, set())
        self.assertEqual(notifications, [])

        model.refresh_identity("Business A")

        self.assertTrue(model.identity_named("Business A").monitor)
        self.assertEqual(model.watch_state.limited_identities, {"Business A"})
        self.assertEqual(notifications, [])

        model.refresh_identity("Business A")

        self.assertTrue(model.identity_named("Business A").monitor)
        self.assertEqual(model.watch_state.limited_identities, set())
        self.assertEqual(len(notifications), 1)

    def test_non_team_limited_snapshot_does_not_enter_pending_notification_list(self):
        notifications = []

        def quota_reader(settings, identity):
            return QuotaSnapshot(
                identity=identity.name,
                plan_type="business",
                primary=RateWindow(100, 10, 300),
                secondary=None,
                credits_has_credits=False,
                credits_unlimited=False,
                reached_type="rate_limit_reached",
            )

        model = GuiViewModel(
            self.make_settings(),
            quota_reader=quota_reader,
            notifier=lambda title, message: notifications.append((title, message)),
        )

        model.refresh_identity("Enterprise")

        self.assertFalse(model.identity_named("Enterprise").monitor)
        self.assertEqual(model.watch_state.limited_identities, set())
        self.assertEqual(notifications, [])


if __name__ == "__main__":
    unittest.main()
