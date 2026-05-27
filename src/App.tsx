import { listen } from "@tauri-apps/api/event";
import * as Dialog from "@radix-ui/react-dialog";
import {
  ArrowLeft,
  CircleHelp,
  FileText,
  FolderOpen,
  KeyRound,
  LogIn,
  Loader2,
  Plus,
  Power,
  RefreshCw,
  Settings,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { modexApi } from "./lib/api";
import type { AppLogEntry, AppSettings, DailyWakeSettings, Identity, SettingsPatch } from "./types";

type View = "accounts" | "settings";
type ActionOptions = {
  applyResult?: (result: unknown) => void;
  reload?: boolean;
  showBusy?: boolean;
  failureNoticeTitle?: string;
  successNotice?: {
    title: string;
    message: string;
  };
};
type QuotaLabel = {
  prefix: string;
  percent?: string;
  suffix?: string;
};
type ToastNoticeState = {
  id: string;
  level: AppLogEntry["level"];
  title: string;
  message: string;
};

const WAKE_THRESHOLD_HELP = {
  primary: "当前5小时用量高于该百分比时跳过唤醒，避免在已经消耗过额度的窗口继续触发。",
  weekly: "周额度剩余低于该百分比时跳过唤醒，优先保护团队账号的长期额度。",
  delta: "唤醒后5小时用量增长超过该百分比会记为异常，方便追溯是否触发了超预期消耗。",
};

function App() {
  const [appState, setAppState] = useState<AppSettings | null>(null);
  const [view, setView] = useState<View>("accounts");
  const [busy, setBusy] = useState<string | null>(null);
  const [refreshEventActive, setRefreshEventActive] = useState(false);
  const [logEntries, setLogEntries] = useState<AppLogEntry[]>([]);
  const [logOpen, setLogOpen] = useState(false);
  const [unreadLogs, setUnreadLogs] = useState(0);
  const [toast, setToast] = useState<ToastNoticeState | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [addAccountDialogOpen, setAddAccountDialogOpen] = useState(false);
  const [apiKeyDialogOpen, setApiKeyDialogOpen] = useState(false);
  const autoImportAttempted = useRef(false);
  const inFlightActions = useRef(new Set<string>());
  const toastTimer = useRef<number | null>(null);

  const appendLog = useCallback(
    (entry: AppLogEntry, unread = true) => {
      setLogEntries((current) => [entry, ...current.filter((item) => item.id !== entry.id)].slice(0, 200));
      if (unread && !logOpen) {
        setUnreadLogs((current) => current + 1);
      }
    },
    [logOpen],
  );

  const appendClientLog = useCallback(
    (title: string, reason: unknown, level: AppLogEntry["level"] = "error") => {
      appendLog(clientLogEntry(title, reason, level));
    },
    [appendLog],
  );

  const showNotice = useCallback(
    (title: string, reason: unknown, level: AppLogEntry["level"]) => {
      const entry = clientLogEntry(title, reason, level);
      appendLog(entry);
      setToast({
        id: entry.id,
        level,
        title,
        message: entry.message,
      });
      if (toastTimer.current) {
        window.clearTimeout(toastTimer.current);
      }
      toastTimer.current = window.setTimeout(() => setToast(null), 3000);
    },
    [appendLog],
  );

  const loadState = useCallback(async () => {
    const next = await modexApi.getAppState();
    setAppState(next);
    setRefreshEventActive(next.isRefreshing);
  }, []);

  const autoImportCurrentIdentity = useCallback(async () => {
    if (autoImportAttempted.current) return;
    autoImportAttempted.current = true;
    const result = await modexApi.importCurrentIdentity();
    if (!result.ok || !result.identity) return;
    if (result.imported) {
      await modexApi.refreshIdentity(result.identity.name);
    }
    await loadState();
  }, [loadState]);

  useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      await loadState();
      const recentLogs = await modexApi.getRecentLogEntries();
      if (!cancelled) {
        setLogEntries((current) => [
          ...current,
          ...recentLogs.filter((entry) => !current.some((item) => item.id === entry.id)),
        ].slice(0, 200));
      }
      if (!cancelled) {
        await autoImportCurrentIdentity();
      }
    };
    bootstrap().catch((reason) => appendClientLog("启动失败", reason));

    const openSettings = listen("modex://open-settings", () => setView("settings"));
    const stateUpdated = listen("modex://state-updated", () => {
      loadState().catch((reason) => appendClientLog("状态刷新失败", reason));
    });
    const refreshStarted = listen("modex://refresh-started", () => setRefreshEventActive(true));
    const refreshFinished = listen("modex://refresh-finished", () => {
      setRefreshEventActive(false);
      loadState().catch((reason) => appendClientLog("状态刷新失败", reason));
    });
    const logEntry = listen("modex://log-entry", (event) => {
      const payload = (event as { payload?: AppLogEntry }).payload;
      if (payload) {
        appendLog(payload);
      }
    });

    return () => {
      cancelled = true;
      openSettings.then((cleanup) => cleanup()).catch(() => undefined);
      stateUpdated.then((cleanup) => cleanup()).catch(() => undefined);
      refreshStarted.then((cleanup) => cleanup()).catch(() => undefined);
      refreshFinished.then((cleanup) => cleanup()).catch(() => undefined);
      logEntry.then((cleanup) => cleanup()).catch(() => undefined);
    };
  }, [appendClientLog, appendLog, autoImportCurrentIdentity, loadState]);

  useEffect(
    () => () => {
      if (toastTimer.current) {
        window.clearTimeout(toastTimer.current);
      }
    },
    [],
  );

  const runAction = useCallback(async (label: string, action: () => Promise<unknown>, options: ActionOptions = {}) => {
    if (inFlightActions.current.has(label)) return;
    const { applyResult, failureNoticeTitle, reload = true, showBusy = true, successNotice } = options;
    inFlightActions.current.add(label);
    if (showBusy) {
      setBusy(label);
    }
    try {
      await waitForNextPaint();
      const result = await action();
      applyResult?.(result);
      if (reload) {
        await loadState();
      }
      if (successNotice) {
        showNotice(successNotice.title, successNotice.message, "info");
      }
    } catch (reason) {
      if (failureNoticeTitle) {
        showNotice(failureNoticeTitle, reason, "error");
      } else {
        appendClientLog("操作失败", reason);
      }
    } finally {
      inFlightActions.current.delete(label);
      if (showBusy) {
        setBusy(null);
      }
    }
  }, [appendClientLog, loadState, showNotice]);

  const pollLoginState = useCallback(
    (pendingIdentity: Identity) => {
      let attempts = 0;
      const tick = async () => {
        attempts += 1;
        try {
          const next = await modexApi.getAppState();
          setAppState(next);
          setRefreshEventActive(next.isRefreshing);
          const matchedIdentity =
            next.identities.find((identity) => identity.codexHome === pendingIdentity.codexHome) ??
            next.identities.find((identity) => identity.name === pendingIdentity.name);
          if (!matchedIdentity) return;
          const stillPending = !matchedIdentity.loggedIn;
          if (stillPending && attempts < 60) {
            window.setTimeout(tick, 2000);
          } else if (matchedIdentity.loggedIn) {
            void runAction("login-refresh", () => modexApi.refreshIdentity(matchedIdentity.name));
          }
        } catch (reason) {
          appendClientLog("登录状态检查失败", reason);
        }
      };
      window.setTimeout(tick, 2000);
    },
    [appendClientLog, runAction],
  );

  const addIdentity = async () => {
    setBusy("add");
    try {
      await waitForNextPaint();
      const identity = await modexApi.addIdentity();
      await loadState();
      void modexApi.loginIdentity(identity.name).catch((reason) => appendClientLog("登录失败", reason));
      pollLoginState(identity);
    } catch (reason) {
      appendClientLog("新增账号失败", reason);
    } finally {
      setBusy(null);
    }
  };

  const addApiKeyIdentity = (accountName: string, apiKey: string, baseUrl: string) =>
    runAction(
      "api-key-login",
      () => modexApi.addApiKeyIdentity(accountName, apiKey, baseUrl.trim() ? baseUrl : null),
      {
        applyResult: (result) => {
          const identity = result as Identity;
          setAppState((current) =>
            current
              ? {
                  ...current,
                  hasCompletedSetup: true,
                  identities: [
                    ...current.identities.filter((item) => item.name !== identity.name),
                    identity,
                  ],
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

  const startBrowserLogin = () => {
    setAddAccountDialogOpen(false);
    addIdentity();
  };

  const startApiKeyLogin = () => {
    setAddAccountDialogOpen(false);
    setApiKeyDialogOpen(true);
  };

  const openIdentityDirectory = (name: string) =>
    runAction("open-dir", () => modexApi.openIdentityDirectory(name), {
      reload: false,
    });

  const reloginIdentity = (identity: Identity) =>
    runAction(
      `login-${identity.name}`,
      async () => {
        await modexApi.loginIdentity(identity.name);
        pollLoginState(identity);
      },
      {
        failureNoticeTitle: "重新登录失败",
        reload: false,
      },
    );

  const switchIdentity = (name: string) =>
    runAction(
      "switch",
      async () => {
        await modexApi.switchIdentity(name);
        setAppState((current) =>
          current
            ? {
                ...current,
                currentIdentityName: name,
                identities: current.identities.map((identity) => ({
                  ...identity,
                  isCurrent: identity.name === name,
                })),
              }
            : current,
        );
      },
      { reload: false },
    );

  const requestDeleteIdentity = (name: string) => {
    setDeleteTarget(name);
  };

  const toggleLog = () => {
    if (!logOpen) {
      setUnreadLogs(0);
    }
    setLogOpen((current) => !current);
  };

  const removeIdentityLocally = useCallback((name: string) => {
    setAppState((current) => {
      if (!current) return current;
      const identities = current.identities.filter((identity) => identity.name !== name);
      return {
        ...current,
        hasCompletedSetup: identities.length > 0 ? current.hasCompletedSetup : false,
        currentIdentityName: current.currentIdentityName === name ? null : current.currentIdentityName,
        identities,
      };
    });
  }, []);

  const confirmDeleteIdentity = () => {
    if (!deleteTarget) return;
    const name = deleteTarget;
    setDeleteTarget(null);
    runAction(
      "delete",
      async () => {
        await modexApi.deleteIdentity(name);
        removeIdentityLocally(name);
      },
      { reload: false },
    );
  };

  if (!appState) {
    return (
      <>
        <main className="boot-screen">
          <Loader2 className="spin" size={28} />
          <span>加载 Modex</span>
        </main>
        <RefreshDialog open={refreshEventActive} />
      </>
    );
  }

  const isSwitching = busy === "switch";
  const isRefreshing = busy === "refresh" || busy === "login-refresh" || refreshEventActive;
  const isSettingsView = view === "settings";

  return (
    <main className="app-shell">
      <section className="workspace">
        <header className={`toolbar ${isSettingsView ? "settings-toolbar" : ""}`}>
          <div className="title-lockup">
            {isSettingsView ? (
              <button className="icon-button back-button" onClick={() => setView("accounts")} aria-label="返回账号">
                <ArrowLeft size={17} />
              </button>
            ) : null}
            <span className="brand-mark" aria-hidden="true">M</span>
            <h1 className="brand-word">Modex</h1>
          </div>
          {!isSettingsView ? (
            <div className="toolbar-actions">
              <button
                className="icon-button"
                onClick={() => runAction("refresh", () => modexApi.refreshAll())}
                disabled={busy !== null}
                aria-busy={isRefreshing}
              >
                <RefreshCw className={isRefreshing ? "spin" : undefined} size={17} />
                刷新全部账号
              </button>
              <button className="primary-button" onClick={() => setAddAccountDialogOpen(true)} disabled={busy !== null}>
                <Plus size={17} />
                新增账号
              </button>
              <LogButton open={logOpen} unread={unreadLogs > 0} onClick={toggleLog} />
              <button
                className="icon-button settings-toggle"
                onClick={() => setView("settings")}
                aria-label="打开全局设置"
                aria-pressed={false}
              >
                <Settings size={18} />
              </button>
            </div>
          ) : null}
        </header>

        <div className={`content-pane ${isSettingsView ? "settings-content" : "accounts-content"}`}>
          {isSettingsView ? (
            <SettingsView
              appState={appState}
              busy={busy !== null}
              onSave={(patch) =>
                runAction("settings", () => modexApi.updateSettings(patch), {
                  applyResult: (next) => setAppState(next as AppSettings),
                  failureNoticeTitle: "全局设置保存失败",
                  reload: false,
                  showBusy: false,
                  successNotice: {
                    title: "全局设置已保存",
                    message: "配置已写入。",
                  },
                })
              }
              onSaveWake={(dailyWake) =>
                runAction("wake-settings", () => modexApi.updateDailyWakeSettings(dailyWake), {
                  applyResult: (next) => setAppState(next as AppSettings),
                  failureNoticeTitle: "唤醒设置保存失败",
                  reload: false,
                  showBusy: false,
                  successNotice: {
                    title: "唤醒设置已保存",
                    message: "每日后台唤醒配置已写入。",
                  },
                })
              }
              onRunWakeNow={(dailyWake) =>
                runAction(
                  "wake-now",
                  async () => {
                    const next = await modexApi.updateDailyWakeSettings(dailyWake);
                    setAppState(next);
                    return modexApi.runDailyWakeNow();
                  },
                  {
                    failureNoticeTitle: "测试唤醒失败",
                    reload: false,
                    successNotice: {
                      title: "测试唤醒已完成",
                      message: "详情已写入日志面板。",
                    },
                  },
                )
              }
            />
          ) : appState.identities.length > 0 ? (
            <AccountWorkbench
              identities={appState.identities}
              busy={busy}
              onSwitch={switchIdentity}
              onRelogin={reloginIdentity}
              onOpenDirectory={openIdentityDirectory}
              onDelete={requestDeleteIdentity}
            />
          ) : (
            <EmptyAccounts onAdd={() => setAddAccountDialogOpen(true)} busy={busy !== null} />
          )}
        </div>
        {!isSettingsView && logOpen ? <LogPanel entries={logEntries} onClose={() => setLogOpen(false)} /> : null}
      </section>
      <RefreshDialog open={isRefreshing || isSwitching} />
      <DeleteConfirmDialog
        accountName={deleteTarget}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={confirmDeleteIdentity}
      />
      <AddAccountDialog
        open={addAccountDialogOpen}
        busy={busy !== null}
        onCancel={() => setAddAccountDialogOpen(false)}
        onBrowserLogin={startBrowserLogin}
        onApiKeyLogin={startApiKeyLogin}
      />
      <ApiKeyDialog
        open={apiKeyDialogOpen}
        busy={busy === "api-key-login"}
        onCancel={() => setApiKeyDialogOpen(false)}
        onSubmit={(accountName, apiKey, baseUrl) => {
          setApiKeyDialogOpen(false);
          addApiKeyIdentity(accountName, apiKey, baseUrl);
        }}
      />
      {toast ? <ToastNotice notice={toast} /> : null}
    </main>
  );
}

function ToastNotice({ notice }: { notice: ToastNoticeState }) {
  return (
    <div className={`toast-notice ${notice.level}`} role={notice.level === "error" ? "alert" : "status"}>
      <strong>{notice.title}</strong>
      <span>{notice.message}</span>
    </div>
  );
}

function LogButton({ open, unread, onClick }: { open: boolean; unread: boolean; onClick: () => void }) {
  return (
    <button className="icon-button log-toggle" onClick={onClick} aria-label={open ? "关闭日志" : "打开日志"} aria-pressed={open}>
      <FileText size={18} />
      {unread ? <span className="unread-dot" aria-hidden="true" /> : null}
    </button>
  );
}

function LogPanel({ entries, onClose }: { entries: AppLogEntry[]; onClose: () => void }) {
  return (
    <aside className="log-panel" role="region" aria-label="日志">
      <div className="log-panel-header">
        <div>
          <h2>运行日志</h2>
          <span>{entries.length} 条</span>
        </div>
        <button className="icon-button log-close" aria-label="关闭日志面板" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="log-list">
        {entries.length > 0 ? (
          entries.map((entry) => (
            <article className={`log-entry ${entry.level}`} key={entry.id}>
              <div className="log-entry-main">
                <strong>{entry.title}</strong>
                <span>{formatLogTime(entry.timestampMillis)}</span>
              </div>
              <p>{entry.message}</p>
              {entry.identityName ? <small>{entry.identityName}</small> : null}
            </article>
          ))
        ) : (
          <p className="log-empty">暂无日志</p>
        )}
      </div>
    </aside>
  );
}

function AccountWorkbench({
  identities,
  busy,
  onSwitch,
  onRelogin,
  onOpenDirectory,
  onDelete,
}: {
  identities: Identity[];
  busy: string | null;
  onSwitch: (name: string) => void;
  onRelogin: (identity: Identity) => void;
  onOpenDirectory: (name: string) => void;
  onDelete: (name: string) => void;
}) {
  return (
    <div className="account-list" role="region" aria-label="账号列表">
      {identities.map((identity) => (
        <AccountRow
          key={identity.name}
          identity={identity}
          busy={busy}
          onSwitch={() => onSwitch(identity.name)}
          onRelogin={() => onRelogin(identity)}
          onOpenDirectory={() => onOpenDirectory(identity.name)}
          onDelete={() => onDelete(identity.name)}
        />
      ))}
    </div>
  );
}

function RefreshDialog({ open }: { open: boolean }) {
  return (
    <Dialog.Root open={open} modal={false}>
      <Dialog.Portal>
        <Dialog.Overlay className="refresh-overlay" />
        <Dialog.Content className="refresh-dialog" aria-describedby={undefined}>
          <Loader2 className="spin" size={24} />
          <Dialog.Title className="sr-only">处理中</Dialog.Title>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function DeleteConfirmDialog({
  accountName,
  onCancel,
  onConfirm,
}: {
  accountName: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog.Root open={Boolean(accountName)} onOpenChange={(open) => (!open ? onCancel() : undefined)}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content className="confirm-dialog" aria-describedby="delete-account-description">
          <Dialog.Title>删除账号</Dialog.Title>
          <Dialog.Description id="delete-account-description">
            确定要删除账号 {accountName} 吗？
          </Dialog.Description>
          <div className="confirm-actions">
            <button className="icon-button" onClick={onCancel}>
              取消
            </button>
            <button className="danger-button confirm-danger" onClick={onConfirm}>
              确认删除
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function AddAccountDialog({
  open,
  busy,
  onCancel,
  onBrowserLogin,
  onApiKeyLogin,
}: {
  open: boolean;
  busy: boolean;
  onCancel: () => void;
  onBrowserLogin: () => void;
  onApiKeyLogin: () => void;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(nextOpen) => (!nextOpen ? onCancel() : undefined)}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content className="add-account-dialog" aria-describedby={undefined}>
          <Dialog.Title>新增账号</Dialog.Title>
          <div className="add-account-options">
            <button className="account-method-button" onClick={onBrowserLogin} disabled={busy}>
              <Plus size={18} />
              浏览器登录
            </button>
            <button className="account-method-button" onClick={onApiKeyLogin} disabled={busy}>
              <KeyRound size={18} />
              API Key 登录
            </button>
          </div>
          <div className="confirm-actions">
            <button className="icon-button" onClick={onCancel} disabled={busy}>
              取消
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function ApiKeyDialog({
  open,
  busy,
  onCancel,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  onCancel: () => void;
  onSubmit: (accountName: string, apiKey: string, baseUrl: string) => void;
}) {
  const [form, setForm] = useState({ accountName: "", apiKey: "", baseUrl: "" });
  useEffect(() => {
    if (!open) {
      setForm({ accountName: "", apiKey: "", baseUrl: "" });
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
            <input
              value={form.accountName}
              onChange={(event) => setForm({ ...form, accountName: event.target.value })}
            />
          </label>
          <label>
            <span>API Key</span>
            <input
              type="password"
              value={form.apiKey}
              onChange={(event) => setForm({ ...form, apiKey: event.target.value })}
            />
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
              onClick={() => onSubmit(form.accountName, form.apiKey, form.baseUrl)}
              disabled={busy || !form.accountName.trim() || !form.apiKey.trim()}
            >
              保存 API Key
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function AccountRow({
  identity,
  busy,
  onSwitch,
  onRelogin,
  onOpenDirectory,
  onDelete,
}: {
  identity: Identity;
  busy: string | null;
  onSwitch: () => void;
  onRelogin: () => void;
  onOpenDirectory: () => void;
  onDelete: () => void;
}) {
  const disabled = busy !== null;
  const canRelogin = identity.authType === "chatGpt" && (identity.loginExpired || !identity.loggedIn);
  const shouldShowQuota = identity.quota.status !== "unknown" && identity.quota.plan !== "企业版";
  const quotaMeters = shouldShowQuota
    ? [
        {
          label: formatQuotaLabel(identity.quota.primaryLabel, identity.quota.primaryResetAt),
          value: identity.quota.primaryPercent,
        },
        {
          label: formatQuotaLabel(identity.quota.secondaryLabel, identity.quota.secondaryResetAt),
          value: identity.quota.secondaryPercent,
        },
      ].filter((meter): meter is { label: QuotaLabel; value: number } => meter.label !== null)
    : [];
  const showQuotaPlaceholder = shouldShowQuota && quotaMeters.length === 0;

  return (
    <article className={`account-card ${identity.isCurrent ? "current" : ""}`} aria-label={`${identity.name} account`}>
      <div className="account-main">
        <div className="account-title-block">
          <div className="account-title-line">
            <span className="account-status-slot">
              <span className={`account-status ${statusTone(identity)}`}>
                <span aria-hidden="true" />
                {statusLabel(identity)}
              </span>
            </span>
            <h3>{identity.name}</h3>
          </div>
        </div>
      </div>

      <div className="quota-stack compact">
        {quotaMeters.length > 0 ? (
          quotaMeters.map((meter) => (
            <QuotaMeter key={`${meter.label.prefix}${meter.label.percent ?? ""}${meter.label.suffix ?? ""}`} label={meter.label} value={meter.value} />
          ))
        ) : showQuotaPlaceholder ? (
          <p className="quota-empty">暂无可显示配额条</p>
        ) : null}
      </div>

      <div className="account-actions" aria-label={`${identity.name} actions`}>
        <button
          className="primary-button"
          aria-label={`切换到 ${identity.name}`}
          title={identity.isCurrent ? "这是当前使用的账号" : undefined}
          onClick={onSwitch}
          disabled={disabled || identity.isCurrent || identity.loginExpired || !identity.loggedIn}
        >
          <Power size={16} />
          切换
        </button>
        {canRelogin ? (
          <button
            className="primary-button"
            aria-label={`重新登录 ${identity.name}`}
            onClick={onRelogin}
            disabled={disabled}
          >
            <LogIn size={16} />
            重新登录
          </button>
        ) : null}
        <button className="icon-button" aria-label={`打开 ${identity.name} 配置目录`} onClick={onOpenDirectory} disabled={disabled}>
          <FolderOpen size={16} />
        </button>
        <button className="danger-button" aria-label={`删除 ${identity.name}`} onClick={onDelete} disabled={disabled}>
          <Trash2 size={16} />
        </button>
      </div>
    </article>
  );
}

function SettingsView({
  appState,
  busy,
  onSave,
  onSaveWake,
  onRunWakeNow,
}: {
  appState: AppSettings;
  busy: boolean;
  onSave: (patch: SettingsPatch) => void;
  onSaveWake: (dailyWake: DailyWakeSettings) => void;
  onRunWakeNow: (dailyWake: DailyWakeSettings) => void;
}) {
  const [form, setForm] = useState({
    codexBinary: appState.codexBinary,
    appName: appState.appName,
    sourceHome: appState.sourceHome,
    pollSeconds: String(appState.pollSeconds),
  });
  const [wakeForm, setWakeForm] = useState({
    enabled: appState.dailyWake.enabled,
    times: wakeTimesFromSettings(appState.dailyWake),
    message: appState.dailyWake.message,
    skipIfPrimaryUsedAbovePercent: String(appState.dailyWake.skipIfPrimaryUsedAbovePercent),
    skipIfWeeklyRemainingBelowPercent: String(appState.dailyWake.skipIfWeeklyRemainingBelowPercent),
    maxPrimaryDeltaPercent: String(appState.dailyWake.maxPrimaryDeltaPercent),
  });

  useEffect(() => {
    setForm({
      codexBinary: appState.codexBinary,
      appName: appState.appName,
      sourceHome: appState.sourceHome,
      pollSeconds: String(appState.pollSeconds),
    });
    setWakeForm({
      enabled: appState.dailyWake.enabled,
      times: wakeTimesFromSettings(appState.dailyWake),
      message: appState.dailyWake.message,
      skipIfPrimaryUsedAbovePercent: String(appState.dailyWake.skipIfPrimaryUsedAbovePercent),
      skipIfWeeklyRemainingBelowPercent: String(appState.dailyWake.skipIfWeeklyRemainingBelowPercent),
      maxPrimaryDeltaPercent: String(appState.dailyWake.maxPrimaryDeltaPercent),
    });
  }, [appState.codexBinary, appState.appName, appState.sourceHome, appState.pollSeconds, appState.dailyWake]);

  const currentWakeSettings = (): DailyWakeSettings => ({
    enabled: wakeForm.enabled,
    time: wakeForm.times[0] ?? "08:30",
    times: wakeForm.times,
    message: wakeForm.message,
    skipIfPrimaryUsedAbovePercent: Number(wakeForm.skipIfPrimaryUsedAbovePercent),
    skipIfWeeklyRemainingBelowPercent: Number(wakeForm.skipIfWeeklyRemainingBelowPercent),
    maxPrimaryDeltaPercent: Number(wakeForm.maxPrimaryDeltaPercent),
    lastRunDate: null,
    lastRunSlots: appState.dailyWake.lastRunSlots ?? [],
  });

  const updateWakeTime = (index: number, value: string) => {
    setWakeForm({
      ...wakeForm,
      times: wakeForm.times.map((time, timeIndex) => (timeIndex === index ? value : time)),
    });
  };

  const addWakeTime = () => {
    setWakeForm({ ...wakeForm, times: [...wakeForm.times, "08:30"] });
  };

  const removeWakeTime = (index: number) => {
    if (wakeForm.times.length <= 1) return;
    setWakeForm({ ...wakeForm, times: wakeForm.times.filter((_, timeIndex) => timeIndex !== index) });
  };

  return (
    <section className="settings-panel">
      <label>
        <span>Codex CLI</span>
        <input value={form.codexBinary} onChange={(event) => setForm({ ...form, codexBinary: event.target.value })} />
      </label>
      <label>
        <span>Codex App</span>
        <input value={form.appName} onChange={(event) => setForm({ ...form, appName: event.target.value })} />
      </label>
      <label>
        <span>Source Home</span>
        <input value={form.sourceHome} onChange={(event) => setForm({ ...form, sourceHome: event.target.value })} />
      </label>
      <label>
        <span>轮询间隔</span>
        <input
          type="number"
          min={10}
          value={form.pollSeconds}
          onChange={(event) => setForm({ ...form, pollSeconds: event.target.value })}
        />
      </label>
      <button
        className="primary-button settings-save"
        onClick={() =>
          onSave({
            codexBinary: form.codexBinary,
            appName: form.appName,
            sourceHome: form.sourceHome,
            pollSeconds: Number(form.pollSeconds),
          })
        }
        disabled={busy}
      >
        保存全局设置
      </button>
      <div className="settings-divider" />
      <div className="switch-field">
        <div className="switch-copy">
          <span>每日后台唤醒</span>
          <small>{wakeForm.enabled ? "已开启" : "已关闭"}</small>
        </div>
        <button
          type="button"
          className="switch-control"
          role="switch"
          aria-checked={wakeForm.enabled}
          aria-label="每日后台唤醒"
          onClick={() => setWakeForm({ ...wakeForm, enabled: !wakeForm.enabled })}
        >
          <span aria-hidden="true" />
        </button>
      </div>
      <div className="wake-times-field">
        <div className="wake-times-header">
          <span>唤醒时间</span>
          <button className="icon-button compact-text-button" type="button" onClick={addWakeTime} disabled={busy}>
            <Plus size={15} />
            新增唤醒时间
          </button>
        </div>
        <div className="wake-time-list">
          {wakeForm.times.map((time, index) => (
            <div className="wake-time-row" key={index}>
              <input
                type="time"
                aria-label={`唤醒时间 ${index + 1}`}
                value={time}
                onChange={(event) => updateWakeTime(index, event.target.value)}
              />
              <button
                className="icon-button wake-time-remove"
                type="button"
                aria-label={`删除唤醒时间 ${index + 1}`}
                onClick={() => removeWakeTime(index)}
                disabled={busy || wakeForm.times.length <= 1}
              >
                <X size={15} />
              </button>
            </div>
          ))}
        </div>
      </div>
      <label>
        <span>唤醒消息</span>
        <input value={wakeForm.message} onChange={(event) => setWakeForm({ ...wakeForm, message: event.target.value })} />
      </label>
      <div className="settings-grid">
        <div className="threshold-field">
          <div className="field-label-row">
            <label htmlFor="wake-primary-threshold">5小时已用大于</label>
            <HelpTip label="5小时已用大于" text={WAKE_THRESHOLD_HELP.primary} />
          </div>
          <input
            id="wake-primary-threshold"
            type="number"
            min={0}
            max={100}
            value={wakeForm.skipIfPrimaryUsedAbovePercent}
            onChange={(event) => setWakeForm({ ...wakeForm, skipIfPrimaryUsedAbovePercent: event.target.value })}
          />
        </div>
        <div className="threshold-field">
          <div className="field-label-row">
            <label htmlFor="wake-weekly-threshold">本周剩余小于</label>
            <HelpTip label="本周剩余小于" text={WAKE_THRESHOLD_HELP.weekly} />
          </div>
          <input
            id="wake-weekly-threshold"
            type="number"
            min={0}
            max={100}
            value={wakeForm.skipIfWeeklyRemainingBelowPercent}
            onChange={(event) => setWakeForm({ ...wakeForm, skipIfWeeklyRemainingBelowPercent: event.target.value })}
          />
        </div>
        <div className="threshold-field">
          <div className="field-label-row">
            <label htmlFor="wake-delta-threshold">异常增长大于</label>
            <HelpTip label="异常增长大于" text={WAKE_THRESHOLD_HELP.delta} />
          </div>
          <input
            id="wake-delta-threshold"
            type="number"
            min={0}
            max={100}
            value={wakeForm.maxPrimaryDeltaPercent}
            onChange={(event) => setWakeForm({ ...wakeForm, maxPrimaryDeltaPercent: event.target.value })}
          />
        </div>
      </div>
      <div className="settings-actions">
        <button className="primary-button settings-save" onClick={() => onSaveWake(currentWakeSettings())} disabled={busy}>
          保存唤醒设置
        </button>
        <button className="icon-button settings-save" onClick={() => onRunWakeNow(currentWakeSettings())} disabled={busy}>
          立即测试唤醒
        </button>
      </div>
    </section>
  );
}

function HelpTip({ label, text }: { label: string; text: string }) {
  return (
    <button type="button" className="help-tip" aria-label={`说明：${label}`} data-tooltip={text} title={text}>
      <CircleHelp size={13} aria-hidden="true" />
    </button>
  );
}

function wakeTimesFromSettings(settings: DailyWakeSettings) {
  return settings.times?.length > 0 ? settings.times : [settings.time || "08:30"];
}

function EmptyAccounts({ onAdd, busy }: { onAdd: () => void; busy: boolean }) {
  return (
    <section className="empty-state">
      <h3>暂无账号</h3>
      <p>添加一个 Codex 身份后，Modex 会为它维护独立登录态。</p>
      <button className="primary-button" onClick={onAdd} disabled={busy}>
        <Plus size={17} />
        新增账号
      </button>
    </section>
  );
}

function QuotaMeter({ label, value }: { label: QuotaLabel; value: number }) {
  return (
    <div className="quota-meter">
      <div className="meter-label">
        <span>
          {label.prefix}
          {label.percent ? <strong className="quota-percent">{label.percent}</strong> : null}
          {label.suffix}
        </span>
      </div>
      <div className="meter-track">
        <span className={`meter-fill ${usageTone(value)}`} style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
      </div>
    </div>
  );
}

function formatQuotaLabel(label: string, resetAt?: number | null): QuotaLabel | null {
  if (!label) return null;
  const normalized = label.replace("每周已用", "本周已用");
  if (!normalized.includes("已用")) return { prefix: normalized };
  const percent = normalized.match(/^(.*?)(\d+%)(.*)$/);
  const time = `（${formatResetTime(resetAt, normalized)}）`;
  if (!percent) return { prefix: normalized, suffix: time };
  return {
    prefix: percent[1],
    percent: percent[2],
    suffix: `${percent[3]}${time}`,
  };
}

function formatResetTime(resetAt: number | null | undefined, label: string) {
  if (!resetAt) return "未知";
  const date = new Date(resetAt * 1000);
  const month = padDatePart(date.getMonth() + 1);
  const day = padDatePart(date.getDate());
  const hour = padDatePart(date.getHours());
  const minute = padDatePart(date.getMinutes());
  if (label.includes("5小时")) return `${hour}:${minute}`;
  return `${month}/${day} ${hour}:${minute}`;
}

function padDatePart(value: number) {
  return String(value).padStart(2, "0");
}

function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    if (typeof window.requestAnimationFrame === "function") {
      window.requestAnimationFrame(() => window.setTimeout(resolve, 0));
      return;
    }
    window.setTimeout(resolve, 0);
  });
}

function clientLogEntry(title: string, reason: unknown, level: AppLogEntry["level"]): AppLogEntry {
  const timestampMillis = Date.now();
  return {
    id: `ui-${timestampMillis}-${Math.random().toString(36).slice(2)}`,
    timestampMillis,
    level,
    source: "ui",
    title,
    message: reason instanceof Error ? reason.message : String(reason),
  };
}

function formatLogTime(timestampMillis: number) {
  const date = new Date(timestampMillis);
  return `${padDatePart(date.getHours())}:${padDatePart(date.getMinutes())}`;
}

function statusLabel(identity: Identity) {
  if (identity.authType === "apiKey" && identity.loggedIn && !identity.loginExpired) return "API Key";
  if (identity.loginExpired || !identity.loggedIn) return "登录失效";
  if (identity.quota.status === "limited") return "配额受限";
  return "可用";
}

function statusTone(identity: Identity) {
  if (identity.loginExpired || !identity.loggedIn) return "expired";
  if (identity.quota.status === "limited") return "limited";
  return "available";
}

function usageTone(value: number) {
  if (value > 80) return "danger";
  if (value > 50) return "warn";
  return "good";
}

export default App;
