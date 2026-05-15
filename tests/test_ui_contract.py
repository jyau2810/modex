from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
APP_SCRIPT = PROJECT_ROOT / "CodexAccountManager.py"


def test_account_list_uses_managed_directory_button_without_custom_editing():
    source = APP_SCRIPT.read_text()

    assert 'text="配置目录"' in source
    assert "_open_identity_directory" in source
    assert "_begin_inline_edit" not in source
    assert "AccountDialog" not in source
    assert 'text="目录"' not in source


def test_account_list_is_scrollable():
    source = APP_SCRIPT.read_text()

    assert "tk.Canvas" in source
    assert "_identity_rows_frame" in source
    assert "ttk.Scrollbar" not in source


def test_account_list_supports_trackpad_scroll_without_scrollbar():
    source = APP_SCRIPT.read_text()

    assert 'bind_all("<MouseWheel>"' in source
    assert "_is_pointer_over_identity_rows" in source
    assert "_identity_scrollbar" not in source
    assert "_schedule_hide_identity_scrollbar" not in source


def test_quota_progress_is_rendered_as_used_bar_not_default_progressbar():
    source = APP_SCRIPT.read_text()
    quota_progress_source = source.split("def _quota_progress", 1)[1].split(
        "def _show_settings_dialog",
        1,
    )[0]

    assert "_used_quota_bar" in source
    assert "create_rectangle" in source
    assert 'background="#eaecf0"' in quota_progress_source
    assert 'sticky="w"' in quota_progress_source
    assert 'sticky="ew"' not in quota_progress_source
    assert "ttk.Progressbar" not in quota_progress_source


def test_quota_cell_skips_empty_limit_rows():
    source = APP_SCRIPT.read_text()
    quota_cell_source = source.split("def _render_quota_cell", 1)[1].split(
        "def _quota_progress",
        1,
    )[0]

    assert "if display.primary_label:" in quota_cell_source


def test_initial_quota_refresh_shows_and_hides_loading_dialog():
    source = APP_SCRIPT.read_text()
    initial_refresh_source = source.split("def _initial_refresh_all_async", 1)[1].split(
        "def _refresh_all_async",
        1,
    )[0]
    after_initial_source = source.split("def _after_initial_refresh", 1)[1].split(
        "def _after_refresh",
        1,
    )[0]

    assert '_show_loading("正在刷新配额...")' in initial_refresh_source
    assert "_hide_loading()" in after_initial_source
