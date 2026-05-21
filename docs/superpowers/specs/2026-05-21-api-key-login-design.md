# API Key Login Design

## Goal

Allow Modex users to create identities that authenticate with an API key and an optional base URL. API key identities are independent from browser-login ChatGPT identities: they do not depend on, reuse, or imply ownership of any existing `auth.json` account.

## Behavior

Users can add an identity through either browser login or API key login. Browser login keeps the existing behavior: Modex creates an isolated `CODEX_HOME`, runs `codex login` with the ChatGPT login method, and later detects the account name from `auth.json`.

API key login asks for a display name, an API key, and an optional base URL. The base URL can be empty for the default provider endpoint. Modex uses the local Codex CLI API-key login path so the key is stored in the identity's isolated `auth.json` with API-key auth mode, not as a ChatGPT browser token. API key identities appear in the account list as usable identities when their isolated `auth.json` exists. The account status labels them as API key identities rather than browser-login accounts.

## Data Model

Each identity records its authentication type, using a default of browser login for existing configs. Browser identities keep using the current fields and `auth.json`-based detection. API key identities store their optional base URL at the identity level, while the API key credential itself lives in that identity's isolated Codex `auth.json`. Switching identities also switches the active API key auth file and base URL configuration.

Settings migration must preserve existing config files without requiring manual edits. Existing identities deserialize as browser-login identities.

## Switching

Switching to a browser-login identity keeps the current flow: Modex synchronizes that identity's browser-login `auth.json` into the source `CODEX_HOME` and launches or activates Codex.

Switching to an API key identity synchronizes that identity's API-key `auth.json` into the source `CODEX_HOME`, then materializes the optional base URL as Codex's `openai_base_url` setting. The API key value must not be shown in the UI after saving. If a base URL is provided, the active Codex configuration includes it; if it is empty, Modex omits the override.

## Quota And Status

Browser-login identities keep their current quota refresh behavior through Codex app-server and `auth.json`.

API key identities are allowed to be current and switchable even when quota data cannot be read. If the current Codex app-server path cannot return quota data for API key authentication, Modex shows quota as unknown or unavailable rather than marking the identity as login expired.

## Error Handling

API key creation validates that the display name and API key are non-empty after trimming. Base URL is optional, but if provided it should be trimmed and stored consistently. Duplicate display names should be resolved with the same unique-name behavior used elsewhere.

Backend errors during API key identity creation or switching should flow through the existing action/log notice path. Deleting an API key identity removes its managed identity directory and clears current identity if needed, matching existing delete behavior.

## Testing

Rust tests cover config migration defaults, API key identity creation, identity view status for API key identities with API-key `auth.json`, and switching behavior that syncs API-key auth while applying optional base URL configuration.

Frontend tests cover the API key add flow, required field validation through backend errors, hidden API key entry, optional base URL submission, account-list status text, and switching to an API key identity.
