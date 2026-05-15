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
