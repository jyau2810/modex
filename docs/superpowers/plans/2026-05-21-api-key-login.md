# API Key Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add independent API key identities with optional Codex `openai_base_url` support.

**Architecture:** Extend the existing identity model with an auth type and optional base URL while keeping existing browser-login identities as the default. API key creation uses `codex login --with-api-key` against the identity's isolated `CODEX_HOME`; switching syncs that identity's `auth.json` and applies or removes a top-level `openai_base_url` override in the active source `config.toml`.

**Update:** API key identities no longer ask for a manual display name. Modex now automatically derives the identity name from account data when available, falls back to a unique API-key name, and skips quota queries for API-key identities.

**Tech Stack:** Rust/Tauri backend, React/Vitest frontend, Codex CLI, JSON config, line-preserving TOML key update for one top-level setting.

---

## File Structure

- Modify `src-tauri/src/core/app_config.rs`: add `IdentityAuthType`, `auth_type`, and `api_base_url` fields with serde defaults.
- Modify `src-tauri/src/core/identity_home.rs`: ensure browser-created identities explicitly use browser auth defaults.
- Modify `src-tauri/src/core/codex.rs`: add `run_api_key_login`, `apply_openai_base_url_config`, and switch-time runtime config application helpers.
- Modify `src-tauri/src/core/engine.rs`: add API key identity creation and include `authType`/`apiBaseUrl` in `IdentityView`.
- Modify `src-tauri/src/commands.rs` and `src-tauri/src/lib.rs`: expose a new `add_api_key_identity` command.
- Modify `src/types.ts`, `src/lib/api.ts`, `src/App.tsx`, and `src/styles.css`: add API key dialog, fields, status label, and API call.
- Modify tests in `src-tauri/tests/core_config.rs`, `src-tauri/tests/core_codex.rs`, `src-tauri/tests/core_engine.rs`, and `src/App.test.tsx`.

---

### Task 1: Identity Model And Config Migration

**Files:**
- Modify: `src-tauri/src/core/app_config.rs`
- Modify: `src-tauri/src/core/identity_home.rs`
- Modify: Rust test struct literals that construct `AppIdentity`
- Test: `src-tauri/tests/core_config.rs`
- Test: `src-tauri/tests/core_engine.rs`

- [ ] **Step 1: Write failing config migration tests**

Add to `src-tauri/tests/core_config.rs`:

```rust
#[test]
fn existing_identities_default_to_browser_auth() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    config
        .write_str(
            r#"{
  "version": 1,
  "codexBinary": "codex",
  "appName": "Codex",
  "pollSeconds": 60,
  "sourceHome": "/Users/alex/.codex",
  "identities": [
    {
      "name": "team@example.com",
      "codexHome": "/Users/alex/.modex/123456789012"
    }
  ]
}"#,
        )
        .unwrap();

    let settings = load_app_settings_from_path(config.path()).unwrap();

    assert_eq!(settings.identities[0].auth_type, IdentityAuthType::ChatGpt);
    assert_eq!(settings.identities[0].api_base_url, None);
}

#[test]
fn api_key_identity_config_roundtrips_auth_type_and_base_url() {
    let identity = AppIdentity {
        name: "API".to_string(),
        codex_home: PathBuf::from("/tmp/api"),
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    };

    let value = serde_json::to_value(&identity).unwrap();
    let decoded: AppIdentity = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(value["authType"], "apiKey");
    assert_eq!(value["apiBaseUrl"], "https://gateway.example/v1");
    assert_eq!(decoded, identity);
}
```

Update the test imports in `core_config.rs` to include `IdentityAuthType`.

- [ ] **Step 2: Run the config tests and verify they fail**

Run:

```bash
cd src-tauri && cargo test --test core_config auth
```

Expected: compile failure mentioning missing `IdentityAuthType` or missing `auth_type`/`api_base_url` fields.

- [ ] **Step 3: Implement identity auth model**

In `src-tauri/src/core/app_config.rs`, add above `AppIdentity`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdentityAuthType {
    #[default]
    ChatGpt,
    ApiKey,
}
```

Update `AppIdentity`:

```rust
pub struct AppIdentity {
    pub name: String,
    pub codex_home: PathBuf,
    #[serde(default)]
    pub monitor: bool,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub auth_type: IdentityAuthType,
    #[serde(default)]
    pub api_base_url: Option<String>,
}
```

In `src-tauri/src/core/identity_home.rs`, ensure `default_new_identity` populates the two new fields:

```rust
auth_type: IdentityAuthType::ChatGpt,
api_base_url: None,
```

Update all existing Rust `AppIdentity { ... }` test literals to include:

```rust
auth_type: IdentityAuthType::ChatGpt,
api_base_url: None,
```

- [ ] **Step 4: Run the config tests and verify they pass**

Run:

```bash
cd src-tauri && cargo test --test core_config
```

Expected: all `core_config` tests pass.

- [ ] **Step 5: Commit**

Run:

```bash
git add src-tauri/src/core/app_config.rs src-tauri/src/core/identity_home.rs src-tauri/tests
git commit -m "feat: model identity auth type"
```

---

### Task 2: API Key Identity Creation

**Files:**
- Modify: `src-tauri/src/core/codex.rs`
- Modify: `src-tauri/src/core/engine.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/core_codex.rs`
- Test: `src-tauri/tests/core_engine.rs`

- [ ] **Step 1: Write failing Codex CLI API-key login test**

Add to `src-tauri/tests/core_codex.rs`:

```rust
#[test]
fn api_key_login_command_reads_key_from_stdin() {
    use modex_lib::core::app_config::{AppIdentity, IdentityAuthType};
    use modex_lib::core::codex::{api_key_login_invocation, ProgramInvocation};

    let settings = AppSettings::default_for_home(PathBuf::from("/tmp/modex-test"));
    let identity = AppIdentity {
        name: "API".to_string(),
        codex_home: PathBuf::from("/tmp/modex-test/.modex/api"),
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    };

    let invocation: ProgramInvocation = api_key_login_invocation(&settings, &identity);

    assert_eq!(invocation.args, vec!["login".to_string(), "--with-api-key".to_string()]);
    assert_eq!(
        invocation.envs,
        vec![(
            "CODEX_HOME".to_string(),
            "/tmp/modex-test/.modex/api".to_string()
        )]
    );
}
```

- [ ] **Step 2: Write failing engine API-key creation tests**

Add to `src-tauri/tests/core_engine.rs`:

```rust
#[test]
fn add_api_key_identity_creates_isolated_api_key_account() {
    use modex_lib::core::app_config::IdentityAuthType;

    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );
    let mut login_home = None;
    let mut login_key = None;

    let identity = engine
        .add_api_key_identity_with_operations(
            "Gateway",
            " sk-test-key ",
            Some(" https://gateway.example/v1 ".to_string()),
            || "123456789012".to_string(),
            |_settings, identity, api_key| {
                login_home = Some(identity.codex_home.clone());
                login_key = Some(api_key.to_string());
                std::fs::create_dir_all(&identity.codex_home).unwrap();
                std::fs::write(
                    identity.codex_home.join("auth.json"),
                    r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test-key"}"#,
                )
                .unwrap();
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(identity.name, "Gateway");
    assert_eq!(identity.auth_type, IdentityAuthType::ApiKey);
    assert_eq!(identity.api_base_url.as_deref(), Some("https://gateway.example/v1"));
    assert!(identity.logged_in);
    assert_eq!(login_home.unwrap(), temp.path().join(".modex/123456789012"));
    assert_eq!(login_key.as_deref(), Some("sk-test-key"));

    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities[0].auth_type, IdentityAuthType::ApiKey);
    assert_eq!(
        saved.identities[0].api_base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
}

#[test]
fn add_api_key_identity_rejects_empty_name_or_key() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );

    let empty_name = engine.add_api_key_identity_with_operations(
        " ",
        "sk-test",
        None,
        || "123456789012".to_string(),
        |_settings, _identity, _api_key| Ok(()),
    );
    let empty_key = engine.add_api_key_identity_with_operations(
        "API",
        " ",
        None,
        || "123456789012".to_string(),
        |_settings, _identity, _api_key| Ok(()),
    );

    assert!(empty_name.unwrap_err().to_string().contains("账号名称不能为空"));
    assert!(empty_key.unwrap_err().to_string().contains("API Key 不能为空"));
    assert!(engine.settings().identities.is_empty());
}

#[test]
fn api_key_quota_errors_do_not_mark_login_expired() {
    use modex_lib::core::app_config::IdentityAuthType;

    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.set_error("Gateway", "unauthorized".to_string());

    let identity = engine.app_state().identities.into_iter().next().unwrap();
    assert!(identity.logged_in);
    assert!(!identity.login_expired);
    assert_eq!(identity.quota.status, "error");
}
```

- [ ] **Step 3: Run the targeted tests and verify they fail**

Run:

```bash
cd src-tauri && cargo test --test core_codex api_key_login_command_reads_key_from_stdin && cargo test --test core_engine api_key
```

Expected: compile failure for missing functions and `IdentityView` fields.

- [ ] **Step 4: Implement API-key login and engine creation**

In `src-tauri/src/core/codex.rs`, expose the invocation and runner:

```rust
pub fn api_key_login_invocation(settings: &AppSettings, identity: &AppIdentity) -> ProgramInvocation {
    ProgramInvocation {
        program: resolve_codex_binary(&settings.codex_binary),
        args: vec!["login".to_string(), "--with-api-key".to_string()],
        envs: build_codex_env(&identity.codex_home),
    }
}

pub fn run_api_key_login(
    settings: &AppSettings,
    identity: &AppIdentity,
    api_key: &str,
) -> ModexResult<()> {
    std::fs::create_dir_all(&identity.codex_home)?;
    let invocation = api_key_login_invocation(settings, identity);
    let mut child = Command::new(invocation.program)
        .args(invocation.args)
        .envs(invocation.envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ModexError::from("codex login stdin is unavailable"))?;
    stdin.write_all(api_key.as_bytes())?;
    stdin.flush()?;
    drop(stdin);
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(ModexError::from("API Key 登录失败"))
    }
}
```

In `src-tauri/src/core/engine.rs`, import `IdentityAuthType` and `run_api_key_login`, then add:

```rust
pub fn add_api_key_identity(
    &mut self,
    display_name: String,
    api_key: String,
    base_url: Option<String>,
) -> ModexResult<IdentityView> {
    self.add_api_key_identity_with_operations(
        &display_name,
        &api_key,
        base_url,
        random_digits,
        run_api_key_login,
    )
}

pub fn add_api_key_identity_with_operations(
    &mut self,
    display_name: &str,
    api_key: &str,
    base_url: Option<String>,
    mut random_digits: impl FnMut() -> String,
    login: impl FnOnce(&AppSettings, &AppIdentity, &str) -> ModexResult<()>,
) -> ModexResult<IdentityView> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(ModexError::from("账号名称不能为空"));
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(ModexError::from("API Key 不能为空"));
    }
    let home = managed_home_root(&self.settings);
    let mut identity = default_new_identity(&home, &mut random_digits)?;
    identity.name = unique_identity_name(
        display_name,
        self.settings.identities.iter().map(|identity| identity.name.as_str()),
    );
    identity.auth_type = IdentityAuthType::ApiKey;
    identity.api_base_url = clean_optional(base_url);
    if let Err(error) = login(&self.settings, &identity, api_key) {
        let _ = fs::remove_dir_all(&identity.codex_home);
        return Err(error);
    }
    self.settings.identities.push(identity.clone());
    self.settings.has_completed_setup = true;
    self.save()?;
    Ok(self.identity_view(&identity))
}
```

Update `IdentityView` with:

```rust
pub auth_type: IdentityAuthType,
pub api_base_url: Option<String>,
```

and set those fields in `identity_view`.

Update `set_error` so browser auth errors still mark browser identities expired, while API key identities keep their login state and surface the refresh error through quota display:

```rust
pub fn set_error(&mut self, name: &str, error: String) {
    let is_api_key_identity = self
        .identity(name)
        .ok()
        .is_some_and(|identity| identity.auth_type == IdentityAuthType::ApiKey);
    if is_login_expired_error(&error) && !is_api_key_identity {
        self.expired_identity_names.insert(name.to_string());
    } else {
        self.expired_identity_names.remove(name);
    }
    self.errors.insert(name.to_string(), error);
}
```

In `src-tauri/src/commands.rs`, add:

```rust
#[tauri::command]
pub async fn add_api_key_identity(
    app: AppHandle,
    display_name: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<IdentityView, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let identity = with_engine(state.inner(), |engine| {
            engine.add_api_key_identity(display_name, api_key, base_url)
        })?;
        refresh_tray(&app);
        Ok(identity)
    })
    .await
}
```

Register the command in `src-tauri/src/lib.rs`.

- [ ] **Step 5: Run targeted backend tests and verify they pass**

Run:

```bash
cd src-tauri && cargo test --test core_codex api_key_login_command_reads_key_from_stdin && cargo test --test core_engine api_key
```

Expected: targeted tests pass.

- [ ] **Step 6: Commit**

Run:

```bash
git add src-tauri/src/core/codex.rs src-tauri/src/core/engine.rs src-tauri/src/commands.rs src-tauri/src/lib.rs src-tauri/tests
git commit -m "feat: add api key identity creation"
```

---

### Task 3: Switch-Time Base URL Configuration

**Files:**
- Modify: `src-tauri/src/core/codex.rs`
- Test: `src-tauri/tests/core_codex.rs`

- [ ] **Step 1: Write failing base URL config helper tests**

Add to `src-tauri/tests/core_codex.rs`:

```rust
#[test]
fn apply_openai_base_url_config_sets_or_removes_top_level_key() {
    use modex_lib::core::codex::apply_openai_base_url_config;

    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.toml");
    config
        .write_str("model = \"gpt-5.2\"\nopenai_base_url = \"https://old.example/v1\"\n")
        .unwrap();

    apply_openai_base_url_config(temp.path(), Some("https://new.example/v1")).unwrap();
    assert_eq!(
        std::fs::read_to_string(config.path()).unwrap(),
        "model = \"gpt-5.2\"\nopenai_base_url = \"https://new.example/v1\"\n"
    );

    apply_openai_base_url_config(temp.path(), None).unwrap();
    assert_eq!(
        std::fs::read_to_string(config.path()).unwrap(),
        "model = \"gpt-5.2\"\n"
    );
}
```

- [ ] **Step 2: Write failing launch preparation test**

Add to `src-tauri/tests/core_codex.rs`:

```rust
#[test]
fn prepare_identity_for_launch_syncs_api_key_auth_and_applies_base_url() {
    use modex_lib::core::app_config::IdentityAuthType;
    use modex_lib::core::codex::prepare_identity_for_launch;

    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::write(source_home.join("config.toml"), "model = \"gpt-5.2\"\n").unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();

    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home.clone();
    let identity = AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    };

    prepare_identity_for_launch(&settings, &identity).unwrap();

    assert_eq!(
        std::fs::read_to_string(source_home.join("auth.json")).unwrap(),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#
    );
    assert_eq!(
        std::fs::read_to_string(source_home.join("config.toml")).unwrap(),
        "model = \"gpt-5.2\"\nopenai_base_url = \"https://gateway.example/v1\"\n"
    );
}
```

- [ ] **Step 3: Run targeted tests and verify they fail**

Run:

```bash
cd src-tauri && cargo test --test core_codex apply_openai_base_url_config && cargo test --test core_codex prepare_identity_for_launch
```

Expected: compile failure for missing `apply_openai_base_url_config` or `prepare_identity_for_launch`.

- [ ] **Step 4: Implement config application**

In `src-tauri/src/core/codex.rs`, add:

```rust
pub fn apply_openai_base_url_config(
    codex_home: &Path,
    base_url: Option<&str>,
) -> ModexResult<()> {
    std::fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut lines = existing
        .lines()
        .filter(|line| !is_openai_base_url_line(line))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        lines.push(format!(
            "openai_base_url = \"{}\"",
            escape_config_value(base_url)
        ));
    }
    let next = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    std::fs::write(config_path, next)?;
    Ok(())
}

fn is_openai_base_url_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == "openai_base_url"
        || trimmed.starts_with("openai_base_url ")
        || trimmed.starts_with("openai_base_url=")
}

pub fn apply_identity_runtime_config(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    apply_openai_base_url_config(&settings.source_home, identity.api_base_url.as_deref())
}

pub fn prepare_identity_for_launch(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    sync_identity_auth(&settings.source_home, &identity.codex_home)?;
    apply_identity_runtime_config(settings, identity)
}
```

Make `escape_config_value` visible inside the module for this helper.

In `open_codex_app_with_operations`, after `sync(&settings.source_home, &identity.codex_home)?;`, call:

```rust
apply_identity_runtime_config(settings, identity)?;
```

This removes stale `openai_base_url` when switching to a browser identity because browser identities have `api_base_url: None`.

- [ ] **Step 5: Run targeted tests and verify they pass**

Run:

```bash
cd src-tauri && cargo test --test core_codex apply_openai_base_url_config && cargo test --test core_codex prepare_identity_for_launch
```

Expected: targeted tests pass.

- [ ] **Step 6: Commit**

Run:

```bash
git add src-tauri/src/core/codex.rs src-tauri/tests/core_codex.rs
git commit -m "feat: apply api key base url on switch"
```

---

### Task 4: Frontend API And Account Status

**Files:**
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/App.tsx`
- Test: `src/App.test.tsx`

- [ ] **Step 1: Write failing frontend tests for API key status and API wrapper**

Add to `src/App.test.tsx`:

```tsx
it("labels api key identities without marking them expired", async () => {
  mockApi.getAppState.mockResolvedValue(
    state({
      identities: [
        {
          name: "Gateway",
          codexHome: "/Users/alex/.modex/api",
          authType: "apiKey",
          apiBaseUrl: "https://gateway.example/v1",
          loggedIn: true,
          loginExpired: false,
          isCurrent: false,
          quota: {
            status: "unknown",
            plan: "API Key",
            primaryLabel: "",
            primaryPercent: 0,
            primaryResetAt: null,
            secondaryLabel: "",
            secondaryPercent: 0,
            secondaryResetAt: null,
            credits: "额度未知",
          },
        },
      ],
      currentIdentityName: null,
    }),
  );

  render(<App />);

  const row = await screen.findByRole("article", { name: /Gateway/ });
  expect(row.querySelector(".account-status")).toHaveTextContent("API Key");
  expect(row.querySelector(".account-status")).not.toHaveTextContent("登录失效");
  expect(within(row).getByRole("button", { name: /切换到 Gateway/ })).not.toBeDisabled();
});
```

Update the `state()` helper identities and any inline `Identity` test literals with `authType: "chatGpt"` and `apiBaseUrl: null`.

- [ ] **Step 2: Run the frontend test and verify it fails**

Run:

```bash
npm test -- --run src/App.test.tsx -t "labels api key identities"
```

Expected: TypeScript or assertion failure because `authType` is not defined or status remains "可用".

- [ ] **Step 3: Implement types, API wrapper, and status label**

In `src/types.ts`, add:

```ts
export type IdentityAuthType = "chatGpt" | "apiKey";
```

Update `Identity`:

```ts
authType: IdentityAuthType;
apiBaseUrl?: string | null;
```

In `src/lib/api.ts`, add:

```ts
addApiKeyIdentity: (displayName: string, apiKey: string, baseUrl?: string | null) =>
  invoke<Identity>("add_api_key_identity", { displayName, apiKey, baseUrl }),
```

In `src/App.tsx`, update status helpers:

```tsx
function statusLabel(identity: Identity) {
  if (identity.authType === "apiKey" && identity.loggedIn && !identity.loginExpired) return "API Key";
  if (identity.loginExpired || !identity.loggedIn) return "登录失效";
  if (identity.quota.status === "limited") return "配额受限";
  return "可用";
}
```

Keep `statusTone` returning `"available"` for usable API key identities.

- [ ] **Step 4: Run frontend status test and verify it passes**

Run:

```bash
npm test -- --run src/App.test.tsx -t "labels api key identities"
```

Expected: targeted test passes.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/types.ts src/lib/api.ts src/App.tsx src/App.test.tsx
git commit -m "feat: surface api key identity status"
```

---

### Task 5: API Key Add Dialog

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/styles.css`
- Test: `src/App.test.tsx`

- [ ] **Step 1: Write failing API key dialog test**

Add to `src/App.test.tsx`:

```tsx
it("adds an api key identity with optional base url", async () => {
  const apiIdentity = {
    name: "Gateway",
    codexHome: "/Users/alex/.modex/api",
    authType: "apiKey" as const,
    apiBaseUrl: "https://gateway.example/v1",
    loggedIn: true,
    loginExpired: false,
    isCurrent: false,
    quota: {
      status: "unknown" as const,
      plan: "API Key",
      primaryLabel: "",
      primaryPercent: 0,
      primaryResetAt: null,
      secondaryLabel: "",
      secondaryPercent: 0,
      secondaryResetAt: null,
      credits: "额度未知",
    },
  };
  mockApi.getAppState
    .mockResolvedValueOnce(state())
    .mockResolvedValueOnce(state({ identities: [...state().identities, apiIdentity] }));
  mockApi.addApiKeyIdentity.mockResolvedValue(apiIdentity);

  render(<App />);

  await screen.findByRole("heading", { name: "Modex", level: 1 });
  await userEvent.click(screen.getByRole("button", { name: "API Key 登录" }));
  await userEvent.type(screen.getByLabelText("账号名称"), "Gateway");
  await userEvent.type(screen.getByLabelText("API Key"), "sk-test-key");
  await userEvent.type(screen.getByLabelText("Base URL"), "https://gateway.example/v1");
  expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "password");
  await userEvent.click(screen.getByRole("button", { name: "保存 API Key" }));

  await waitFor(() =>
    expect(mockApi.addApiKeyIdentity).toHaveBeenCalledWith(
      "Gateway",
      "sk-test-key",
      "https://gateway.example/v1",
    ),
  );
  expect(await screen.findByRole("article", { name: /Gateway/ })).toBeInTheDocument();
  expect(screen.queryByDisplayValue("sk-test-key")).not.toBeInTheDocument();
});
```

Add `addApiKeyIdentity: vi.fn()` to the mock API.

- [ ] **Step 2: Run the dialog test and verify it fails**

Run:

```bash
npm test -- --run src/App.test.tsx -t "adds an api key identity"
```

Expected: failure because the button/dialog do not exist.

- [ ] **Step 3: Implement dialog state and submit handler**

In `App`, add:

```tsx
const [apiKeyDialogOpen, setApiKeyDialogOpen] = useState(false);
```

Add a handler:

```tsx
const addApiKeyIdentity = (displayName: string, apiKey: string, baseUrl: string) =>
  runAction(
    "api-key-login",
    () => modexApi.addApiKeyIdentity(displayName, apiKey, baseUrl.trim() ? baseUrl : null),
    {
      applyResult: (result) => {
        const identity = result as Identity;
        setAppState((current) =>
          current
            ? {
                ...current,
                hasCompletedSetup: true,
                identities: [...current.identities.filter((item) => item.name !== identity.name), identity],
              }
            : current,
        );
      },
      failureNoticeTitle: "API Key 登录失败",
      reload: true,
      successNotice: {
        title: "API Key 账号已添加",
        message: "已保存为独立身份。",
      },
    },
  );
```

Add a toolbar button:

```tsx
<button className="icon-button" onClick={() => setApiKeyDialogOpen(true)} disabled={busy !== null}>
  <KeyRound size={17} />
  API Key 登录
</button>
```

Import `KeyRound` from `lucide-react`.

Render:

```tsx
<ApiKeyDialog
  open={apiKeyDialogOpen}
  busy={busy === "api-key-login"}
  onCancel={() => setApiKeyDialogOpen(false)}
  onSubmit={(displayName, apiKey, baseUrl) => {
    setApiKeyDialogOpen(false);
    addApiKeyIdentity(displayName, apiKey, baseUrl);
  }}
/>
```

- [ ] **Step 4: Implement `ApiKeyDialog`**

Add below `DeleteConfirmDialog`:

```tsx
function ApiKeyDialog({
  open,
  busy,
  onCancel,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  onCancel: () => void;
  onSubmit: (displayName: string, apiKey: string, baseUrl: string) => void;
}) {
  const [form, setForm] = useState({ displayName: "", apiKey: "", baseUrl: "" });
  useEffect(() => {
    if (!open) {
      setForm({ displayName: "", apiKey: "", baseUrl: "" });
    }
  }, [open]);
  return (
    <Dialog.Root open={open} onOpenChange={(nextOpen) => (!nextOpen ? onCancel() : undefined)}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content className="api-key-dialog" aria-describedby={undefined}>
          <Dialog.Title>API Key 登录</Dialog.Title>
          <label>
            <span>账号名称</span>
            <input value={form.displayName} onChange={(event) => setForm({ ...form, displayName: event.target.value })} />
          </label>
          <label>
            <span>API Key</span>
            <input type="password" value={form.apiKey} onChange={(event) => setForm({ ...form, apiKey: event.target.value })} />
          </label>
          <label>
            <span>Base URL</span>
            <input value={form.baseUrl} onChange={(event) => setForm({ ...form, baseUrl: event.target.value })} />
          </label>
          <div className="confirm-actions">
            <button className="icon-button" onClick={onCancel} disabled={busy}>
              取消
            </button>
            <button
              className="primary-button confirm-danger"
              onClick={() => onSubmit(form.displayName, form.apiKey, form.baseUrl)}
              disabled={busy}
            >
              保存 API Key
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
```

- [ ] **Step 5: Add dialog styling**

In `src/styles.css`, extend the existing modal styles:

```css
.api-key-dialog {
  position: fixed;
  top: 50%;
  left: 50%;
  z-index: 31;
  width: min(430px, calc(100vw - 32px));
  transform: translate(-50%, -50%);
  display: grid;
  gap: 14px;
  border-radius: 8px;
  padding: 18px;
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 18px 70px rgba(0, 0, 0, 0.24);
}

.api-key-dialog label {
  display: grid;
  gap: 7px;
  color: var(--muted);
  font-size: 13px;
  font-weight: 800;
}
```

- [ ] **Step 6: Run dialog test and verify it passes**

Run:

```bash
npm test -- --run src/App.test.tsx -t "adds an api key identity"
```

Expected: targeted test passes.

- [ ] **Step 7: Commit**

Run:

```bash
git add src/App.tsx src/styles.css src/App.test.tsx
git commit -m "feat: add api key login dialog"
```

---

### Task 6: Full Verification And Docs

**Files:**
- Modify: `README.md`
- Verify: Rust tests, frontend tests, and TypeScript/Vite build.

- [ ] **Step 1: Add README note**

Update the feature list in `README.md` to include:

```markdown
- API Key 身份：支持用 API Key 新增独立身份，并可为该身份配置可选 Base URL。
```

Update the account switching note to explain:

```markdown
浏览器登录身份切换时同步 `auth.json`；API Key 身份切换时同步其 API-key `auth.json`，并按需写入 `openai_base_url`。
```

- [ ] **Step 2: Run Rust tests**

Run:

```bash
cd src-tauri && cargo test
```

Expected: all Rust tests pass.

- [ ] **Step 3: Run frontend tests**

Run:

```bash
npm test -- --run
```

Expected: all Vitest tests pass.

- [ ] **Step 4: Run production build**

Run:

```bash
npm run build
```

Expected: TypeScript and Vite build pass.

- [ ] **Step 5: Commit docs and any final fixes**

Run:

```bash
git add README.md src src-tauri
git commit -m "docs: document api key identities"
```
