# Daily Team Wake Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a daily, background-only team-account wake job with quota thresholds, audit logs, and an in-app collapsible log panel.

**Architecture:** Extend persisted settings with a `dailyWake` object and add a focused Rust wake module for threshold decisions, audit entries, and scheduler helpers. Expose wake settings and recent logs through Tauri commands, then replace React banner state with a toolbar log panel that consumes frontend and backend log entries.

**Tech Stack:** Rust, Tauri commands, serde JSON/JSONL, React, TypeScript, Vitest, Cargo tests.

---

## File Structure

- Modify `src-tauri/src/core/app_config.rs` for persisted wake settings defaults.
- Create `src-tauri/src/core/wake.rs` for wake thresholds, decisions, audit log entries, and command execution helpers.
- Modify `src-tauri/src/core/mod.rs` to export the wake module.
- Modify `src-tauri/src/commands.rs` for Tauri commands, scheduler startup, and backend log events.
- Modify `src-tauri/src/lib.rs` to register commands and start the scheduler.
- Modify `src/types.ts` for wake settings and log entry types.
- Modify `src/lib/api.ts` for new commands.
- Modify `src/App.tsx` to add settings controls, log state, log panel, and remove banner rendering.
- Modify `src/styles.css` for the log button, red dot, panel, and settings grouping.
- Add or modify Rust and React tests before production code for each behavior.

## Tasks

### Task 1: Persist Wake Settings

- [ ] Add failing Rust tests in `src-tauri/tests/core_config.rs` proving missing config loads default `dailyWake` values and custom values round-trip.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml core_config` and verify the new tests fail because `dailyWake` does not exist.
- [ ] Add `DailyWakeSettings` to `src-tauri/src/core/app_config.rs` with defaults: disabled, `08:30`, message `Good morning`, skip primary above `3`, skip weekly remaining below `20`, max primary delta `3`, last run date `None`.
- [ ] Re-run the config tests and verify they pass.

### Task 2: Wake Decision Logic

- [ ] Add failing Rust unit tests in `src-tauri/src/core/wake.rs` for team-only filtering, threshold skips, unknown quota skips, daily duplicate skips, and circuit-breaker delta.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml wake` and verify the tests fail because the module is missing.
- [ ] Implement `WakeDecision`, `WakeSkipReason`, `WakeThresholds`, `should_wake_identity`, and `primary_delta_exceeds_limit` in `src-tauri/src/core/wake.rs`.
- [ ] Re-run the wake tests and verify they pass.

### Task 3: Audit Log Persistence

- [ ] Add failing Rust tests for JSONL audit append/read behavior using a temporary data directory.
- [ ] Run the targeted wake tests and verify they fail because logging helpers are missing.
- [ ] Implement `WakeAuditEntry`, `append_wake_log_entry`, and `read_recent_wake_log_entries` with newest-first reads.
- [ ] Re-run the targeted wake tests and verify they pass.

### Task 4: Backend Commands and Scheduler

- [ ] Add failing command tests for updating wake settings and deciding a scheduler run should fire once per local day.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml commands` and verify the new tests fail.
- [ ] Add `update_wake_settings`, `get_recent_log_entries`, and `start_daily_wake_scheduler` in `src-tauri/src/commands.rs`; register commands in `src-tauri/src/lib.rs`.
- [ ] Re-run command tests and verify they pass.

### Task 5: Frontend Types and Settings

- [ ] Add failing Vitest coverage that settings renders daily wake controls with defaults and saves edited thresholds through `updateWakeSettings`.
- [ ] Run `npm test -- src/App.test.tsx` and verify the test fails.
- [ ] Add TypeScript types and API calls in `src/types.ts` and `src/lib/api.ts`.
- [ ] Update `SettingsView` in `src/App.tsx` to render the wake controls.
- [ ] Re-run the frontend test and verify it passes.

### Task 6: Log Panel

- [ ] Add failing Vitest coverage that action failures appear in a collapsed log panel, show a red dot, and do not render a banner under the toolbar.
- [ ] Run `npm test -- src/App.test.tsx` and verify the test fails.
- [ ] Add log state, backend log event listening, toolbar log button, collapsible panel, unread tracking, and remove banner rendering from `src/App.tsx`.
- [ ] Add styles in `src/styles.css`.
- [ ] Re-run the frontend test and verify it passes.

### Task 7: Full Verification

- [ ] Run `npm test`.
- [ ] Run `npm run build`.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml`.
- [ ] Run `python3 -m unittest tests/test_app_packaging.py`.
- [ ] Fix any failures with the same red-green discipline.
