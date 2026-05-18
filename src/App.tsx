import { listen } from "@tauri-apps/api/event";
import * as Dialog from "@radix-ui/react-dialog";
import {
  AlertCircle,
  ArrowLeft,
  FolderOpen,
  Loader2,
  Plus,
  Power,
  RefreshCw,
  Settings,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { modexApi } from "./lib/api";
import type { AppSettings, Identity, SettingsPatch } from "./types";

type View = "accounts" | "settings";
type ActionOptions = {
  reload?: boolean;
};
type QuotaLabel = {
  prefix: string;
  percent?: string;
  suffix?: string;
};

function App() {
  const [appState, setAppState] = useState<AppSettings | null>(null);
  const [view, setView] = useState<View>("accounts");
  const [busy, setBusy] = useState<string | null>(null);
  const [refreshEventActive, setRefreshEventActive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const autoImportAttempted = useRef(false);

  const loadState = useCallback(async () => {
    const next = await modexApi.getAppState();
    setAppState(next);
    setRefreshEventActive(next.isRefreshing);
  }, []);

  const autoImportCurrentIdentity = useCallback(async () => {
    if (autoImportAttempted.current) return;
    autoImportAttempted.current = true;
    const result = await modexApi.importCurrentIdentity();
    if (!result.ok || !result.imported) return;
    if (result.identity) {
      await modexApi.refreshIdentity(result.identity.name);
    }
    await loadState();
  }, [loadState]);

  useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      await loadState();
      if (!cancelled) {
        await autoImportCurrentIdentity();
      }
    };
    bootstrap().catch((reason) => setError(String(reason)));

    const openSettings = listen("modex://open-settings", () => setView("settings"));
    const stateUpdated = listen("modex://state-updated", () => {
      loadState().catch((reason) => setError(String(reason)));
    });
    const refreshStarted = listen("modex://refresh-started", () => setRefreshEventActive(true));
    const refreshFinished = listen("modex://refresh-finished", () => {
      setRefreshEventActive(false);
      loadState().catch((reason) => setError(String(reason)));
    });

    return () => {
      cancelled = true;
      openSettings.then((cleanup) => cleanup()).catch(() => undefined);
      stateUpdated.then((cleanup) => cleanup()).catch(() => undefined);
      refreshStarted.then((cleanup) => cleanup()).catch(() => undefined);
      refreshFinished.then((cleanup) => cleanup()).catch(() => undefined);
    };
  }, [autoImportCurrentIdentity, loadState]);

  const runAction = useCallback(async (label: string, action: () => Promise<unknown>, options: ActionOptions = {}) => {
    const { reload = true } = options;
    setBusy(label);
    setError(null);
    try {
      await action();
      if (reload) {
        await loadState();
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }, [loadState]);

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
          setError(String(reason));
        }
      };
      window.setTimeout(tick, 2000);
    },
    [runAction],
  );

  const addIdentity = async () => {
    setBusy("add");
    setError(null);
    try {
      const identity = await modexApi.addIdentity();
      await loadState();
      void modexApi.loginIdentity(identity.name).catch((reason) => setError(String(reason)));
      pollLoginState(identity);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  const openIdentityDirectory = (name: string) =>
    runAction("open-dir", () => modexApi.openIdentityDirectory(name), {
      reload: false,
    });

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
    setError(null);
    setDeleteTarget(name);
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
              <button className="primary-button" onClick={addIdentity} disabled={busy !== null}>
                <Plus size={17} />
                新增账号
              </button>
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

        {error ? (
          <div className="banner error">
            <AlertCircle size={17} />
            {error}
          </div>
        ) : null}

        <div className={`content-pane ${isSettingsView ? "settings-content" : "accounts-content"}`}>
          {isSettingsView ? (
            <SettingsView
              appState={appState}
              busy={busy !== null}
              onSave={(patch) => runAction("settings", () => modexApi.updateSettings(patch))}
            />
          ) : appState.identities.length > 0 ? (
            <AccountWorkbench
              identities={appState.identities}
              busy={busy}
              onSwitch={switchIdentity}
              onOpenDirectory={openIdentityDirectory}
              onDelete={requestDeleteIdentity}
            />
          ) : (
            <EmptyAccounts onAdd={addIdentity} busy={busy !== null} />
          )}
        </div>
      </section>
      <RefreshDialog open={isRefreshing} />
      <DeleteConfirmDialog
        accountName={deleteTarget}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={confirmDeleteIdentity}
      />
    </main>
  );
}

function AccountWorkbench({
  identities,
  busy,
  onSwitch,
  onOpenDirectory,
  onDelete,
}: {
  identities: Identity[];
  busy: string | null;
  onSwitch: (name: string) => void;
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
          <Dialog.Title>正在刷新账号信息</Dialog.Title>
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

function AccountRow({
  identity,
  busy,
  onSwitch,
  onOpenDirectory,
  onDelete,
}: {
  identity: Identity;
  busy: string | null;
  onSwitch: () => void;
  onOpenDirectory: () => void;
  onDelete: () => void;
}) {
  const disabled = busy !== null;
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
}: {
  appState: AppSettings;
  busy: boolean;
  onSave: (patch: SettingsPatch) => void;
}) {
  const [form, setForm] = useState({
    codexBinary: appState.codexBinary,
    appName: appState.appName,
    sourceHome: appState.sourceHome,
    pollSeconds: String(appState.pollSeconds),
  });

  useEffect(() => {
    setForm({
      codexBinary: appState.codexBinary,
      appName: appState.appName,
      sourceHome: appState.sourceHome,
      pollSeconds: String(appState.pollSeconds),
    });
  }, [appState.codexBinary, appState.appName, appState.sourceHome, appState.pollSeconds]);

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
    </section>
  );
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

function statusLabel(identity: Identity) {
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
