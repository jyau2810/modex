# Daily Team Wake Design

## Goal

Allow Modex to run a daily, background-only wake action for logged-in team accounts so their Codex five-hour usage window can be triggered intentionally, with conservative quota thresholds and complete auditability.

## Behavior

Users can enable or disable the daily wake feature, choose a local wall-clock time, and configure the short wake message. The default message is `Good morning`. The app runs the wake job only while Modex is running in the tray or foreground.

Before sending any wake message, Modex refreshes the account quota and evaluates guardrails. It wakes only accounts that are logged in, not expired, team plan accounts, not already processed today, and within thresholds. The default thresholds are:

- skip when five-hour usage is greater than 3%
- skip when weekly remaining quota is below 20%

Unknown quota, refresh failures, expired logins, limited accounts, and non-team plans are skipped. Every skipped decision is logged with the observed quota and threshold values.

## Execution

The wake action is background-only. It must not switch the visible Codex desktop account or open the Codex app. The preferred execution path is the Codex app-server protocol using the identity's isolated `CODEX_HOME`, because it can expose turn lifecycle and usage notifications. If the local Codex version cannot provide enough usage data through app-server, Modex may use `codex exec --ephemeral` with JSON event logging as a fallback.

The wake prompt is constrained: it uses a fixed short user message and internal instructions that require an extremely short reply and prohibit tool use, file reads, project analysis, command execution, or broad reasoning. Execution uses an empty temporary working directory, a timeout, and no project context.

## Audit Log

Every wake decision writes a structured JSONL entry under the Modex app data directory. Entries include the run id, account name, plan, timestamps, decision, reason, thresholds, quota before and after when available, prompt template version, output summary, exit status, timeout flag, and error message.

The in-app notification surface is replaced by a collapsible log panel. The toolbar shows a log icon. The panel is closed by default. New log entries show a red dot on the icon until the panel is opened. Existing in-app banners are removed; frontend action failures, backend wake entries, refresh failures, switch failures, login expiry notices, and quota recovery notices all appear in the log panel. OS-level notifications can remain for important background events.

## Safety

The wake scheduler runs at most once per local day. It records the last run date in settings after a job starts so repeated loops do not duplicate work. If a wake response is too long, not the expected short acknowledgement, times out, or increases five-hour usage by more than 3 percentage points, Modex stops processing remaining accounts and logs a circuit-breaker event.

## Testing

Rust tests cover default settings migration, threshold decisions, daily run eligibility, JSONL audit entries, and scheduler helpers. Frontend tests cover the log icon red dot, collapsible panel, removal of banner errors, and settings fields for wake configuration.
