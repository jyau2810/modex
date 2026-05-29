import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { AppSettings } from "./types";

const mockApi = vi.hoisted(() => ({
  getAppState: vi.fn(),
  addIdentity: vi.fn(),
  addApiKeyIdentity: vi.fn(),
  importCurrentIdentity: vi.fn(),
  deleteIdentity: vi.fn(),
  switchIdentity: vi.fn(),
  loginIdentity: vi.fn(),
  refreshIdentity: vi.fn(),
  refreshAll: vi.fn(),
  updateSettings: vi.fn(),
  updateDailyWakeSettings: vi.fn(),
  runDailyWakeNow: vi.fn(),
  getRecentLogEntries: vi.fn(),
  openIdentityDirectory: vi.fn(),
  installCodexPlugins: vi.fn(),
}));

const eventMocks = vi.hoisted(() => ({
  listeners: new Map<string, (event?: unknown) => void>(),
  listen: vi.fn(async (eventName: string, handler: (event?: unknown) => void) => {
    eventMocks.listeners.set(eventName, handler);
    return vi.fn();
  }),
}));

vi.mock("./lib/api", () => ({
  modexApi: mockApi,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

function state(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    codexBinary: "codex",
    appName: "Codex",
    pollSeconds: 60,
    sourceHome: "/Users/alex/.codex",
    hasCompletedSetup: true,
    currentIdentityName: "team@example.com",
    dailyWake: {
      enabled: false,
      time: "08:30",
      times: ["08:30"],
      message: "Good morning",
      skipIfPrimaryUsedAbovePercent: 3,
      skipIfWeeklyRemainingBelowPercent: 20,
      maxPrimaryDeltaPercent: 3,
      lastRunDate: null,
      lastRunSlots: [],
    },
    isRefreshing: false,
    identities: [
      {
        name: "team@example.com",
        codexHome: "/Users/alex/.modex/123456789012",
        authType: "chatGpt",
        apiBaseUrl: null,
        loggedIn: true,
        loginExpired: false,
        isCurrent: true,
        quota: {
          status: "available",
          plan: "团队版",
          primaryLabel: "5小时已用 42%",
          primaryPercent: 42,
          primaryResetAt: 1770000000,
          secondaryLabel: "每周已用 68%",
          secondaryPercent: 68,
          secondaryResetAt: 1770036000,
          credits: "额度可用",
        },
      },
      {
        name: "backup@example.com",
        codexHome: "/Users/alex/.modex/999999999999",
        authType: "chatGpt",
        apiBaseUrl: null,
        loggedIn: false,
        loginExpired: true,
        isCurrent: false,
        quota: {
          status: "limited",
          plan: "个人版",
          primaryLabel: "",
          primaryPercent: 0,
          primaryResetAt: null,
          secondaryLabel: "每周已用 80%",
          secondaryPercent: 80,
          secondaryResetAt: 1770039600,
          credits: "无额外额度",
        },
      },
      {
        name: "unknown@example.com",
        codexHome: "/Users/alex/.modex/555555555555",
        authType: "chatGpt",
        apiBaseUrl: null,
        loggedIn: true,
        loginExpired: false,
        isCurrent: false,
        quota: {
          status: "unknown",
          plan: "未知计划",
          primaryLabel: "Unknown quota",
          primaryPercent: 0,
          primaryResetAt: null,
          secondaryLabel: "",
          secondaryPercent: 0,
          secondaryResetAt: null,
          credits: "等待配额数据",
        },
      },
    ],
    ...overrides,
  };
}

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventMocks.listeners.clear();
    mockApi.importCurrentIdentity.mockResolvedValue({
      ok: false,
      message: "当前 Codex 尚未登录，无法导入。",
      identity: null,
      imported: false,
    });
    mockApi.getRecentLogEntries.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders an empty account state with an add action", async () => {
    mockApi.getAppState.mockResolvedValue(state({ identities: [], currentIdentityName: null }));

    render(<App />);

    expect(await screen.findByRole("heading", { name: "暂无账号", level: 3 })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /新增账号/ }).length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: /导入当前账号/ })).not.toBeInTheDocument();
  });

  it("renders accounts as the main workbench without selected-account detail controls", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    const teamRow = await screen.findByRole("article", { name: /team@example.com/ });
    const titleLine = teamRow.querySelector(".account-title-line");
    expect(titleLine).toHaveTextContent("可用team@example.com");
    expect(within(teamRow).queryByText("正在使用")).not.toBeInTheDocument();
    expect(within(teamRow).queryByText("团队版")).not.toBeInTheDocument();
    expect(within(teamRow).queryByText("额度可用")).not.toBeInTheDocument();
    expect(teamRow).toHaveTextContent(/5小时已用 42%（\d{2}:\d{2}）/);
    expect(teamRow).toHaveTextContent(/本周已用 68%（\d{2}\/\d{2} \d{2}:\d{2}）/);
    expect(within(teamRow).getByText("42%").tagName).toBe("STRONG");
    expect(within(teamRow).getByText("68%").tagName).toBe("STRONG");
    expect(within(teamRow).queryByText(/刷新于/)).not.toBeInTheDocument();
    expect(teamRow).not.toHaveTextContent(/5小时已用 42%（\d{4}\//);
    expect(teamRow).not.toHaveTextContent(/本周已用 68%（\d{4}\//);
    expect(within(teamRow).queryByText(/更新于/)).not.toBeInTheDocument();
    expect(within(teamRow).getByRole("button", { name: /切换到 team@example.com/ })).toHaveAttribute(
      "title",
      "当前正在使用这个账号",
    );
    expect(within(teamRow).getByRole("button", { name: /打开 team@example.com 配置目录/ })).toBeInTheDocument();
    expect(within(teamRow).getByRole("button", { name: /打开 team@example.com 配置目录/ })).not.toHaveTextContent("目录");
    expect(within(teamRow).getByRole("button", { name: /删除 team@example.com/ })).not.toHaveTextContent("删除");

    const backupRow = screen.getByRole("article", { name: /backup@example.com/ });
    expect(backupRow.querySelector(".account-status")).toHaveTextContent("需重新登录");
    expect(within(backupRow).queryByText("个人版")).not.toBeInTheDocument();
    expect(within(backupRow).queryByText("无额外额度")).not.toBeInTheDocument();
    expect(backupRow).toHaveTextContent(/本周已用 80%（\d{2}\/\d{2} \d{2}:\d{2}）/);
    expect(within(backupRow).queryByText(/^0%$/)).not.toBeInTheDocument();

    const unknownRow = screen.getByRole("article", { name: /unknown@example.com/ });
    expect(unknownRow.querySelector(".account-status")).toHaveTextContent("可用");
    expect(within(unknownRow).queryByText("Unknown quota")).not.toBeInTheDocument();
    expect(within(unknownRow).queryByText("暂无可展示的额度信息")).not.toBeInTheDocument();
    expect(unknownRow.querySelector(".quota-meter")).not.toBeInTheDocument();

    expect(screen.queryByText("快捷操作")).not.toBeInTheDocument();
    expect(within(backupRow).getByRole("button", { name: /重新登录 backup@example.com/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /刷新配额/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: /监控配额恢复/ })).not.toBeInTheDocument();
  });

  it("opens browser login again for an expired account", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.loginIdentity.mockResolvedValue({ ok: true, message: "已打开浏览器登录：backup@example.com" });

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /重新登录 backup@example.com/ }));

    await waitFor(() => expect(mockApi.loginIdentity).toHaveBeenCalledWith("backup@example.com"));
  });

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
    expect(row.querySelector(".account-status")).not.toHaveTextContent("需重新登录");
    expect(within(row).getByRole("button", { name: /切换到 Gateway/ })).not.toBeDisabled();
  });

  it("shows an api key identity as the current account", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        currentIdentityName: "Gateway",
        identities: [
          {
            name: "Gateway",
            codexHome: "/Users/alex/.modex/api",
            authType: "apiKey",
            apiBaseUrl: "https://gateway.example/v1",
            loggedIn: true,
            loginExpired: false,
            isCurrent: true,
            quota: {
              status: "unknown",
              plan: "计划未知",
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
      }),
    );

    render(<App />);

    const row = await screen.findByRole("article", { name: /Gateway/ });
    expect(row).toHaveClass("current");
    expect(within(row).getByRole("button", { name: /切换到 Gateway/ })).toBeDisabled();
    expect(within(row).getByRole("button", { name: /切换到 Gateway/ })).toHaveAttribute(
      "title",
      "当前正在使用这个账号",
    );
  });

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
        plan: "计划未知",
        primaryLabel: "额度未知",
        primaryPercent: 0,
        primaryResetAt: null,
        secondaryLabel: "额度未知",
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
    expect(screen.queryByRole("button", { name: "API Key 登录" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /新增账号/ }));
    const addDialog = await screen.findByRole("dialog", { name: "新增账号" });
    await userEvent.click(within(addDialog).getByRole("button", { name: "API Key 登录" }));
    await userEvent.type(screen.getByLabelText("账号名"), "Gateway");
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
    const row = await screen.findByRole("article", { name: /Gateway/ });
    expect(row.querySelector(".account-status")).toHaveTextContent("API Key");
    expect(row).not.toHaveTextContent("5小时已用");
    expect(row.querySelector(".quota-meter")).not.toBeInTheDocument();
    expect(mockApi.refreshIdentity).not.toHaveBeenCalled();
    expect(screen.queryByDisplayValue("sk-test-key")).not.toBeInTheDocument();
  });

  it("renders the shell without a left sidebar or legacy workspace labels", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    const { container } = render(<App />);

    expect(await screen.findByRole("heading", { name: "Modex", level: 1 })).toBeInTheDocument();
    expect(container.querySelector(".brand-mark")).toHaveTextContent("M");
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
    expect(screen.queryByText(/Codex account workspace/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /Account workbench/i })).not.toBeInTheDocument();

    const settingsButton = screen.getByRole("button", { name: "打开全局设置" });
    expect(settingsButton).toBeInTheDocument();
    expect(settingsButton).toHaveAttribute("aria-pressed", "false");
    expect(settingsButton).not.toHaveTextContent("设置");
  });

  it("keeps only the account list in a scroll container", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    const accountList = await screen.findByRole("region", { name: "账号列表" });
    expect(accountList).toHaveClass("account-list");

    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    expect(styles).toMatch(/\.account-list\s*{[^}]*overflow:\s*auto/s);
    expect(styles).toMatch(/\.settings-content\s*{[^}]*overflow:\s*auto/s);
  });

  it("switches accounts from the workbench row action", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: state().identities.map((identity) =>
          identity.name === "backup@example.com" ? { ...identity, loggedIn: true, loginExpired: false } : identity,
        ),
      }),
    );
    mockApi.switchIdentity.mockResolvedValue({ ok: true, message: "switched" });

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /切换到 backup@example.com/ }));

    await waitFor(() => expect(mockApi.switchIdentity).toHaveBeenCalledWith("backup@example.com"));
    expect(mockApi.getAppState).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("article", { name: /backup@example.com/ })).toHaveClass("current");
    expect(within(screen.getByRole("article", { name: /backup@example.com/ })).getByRole("button", { name: /切换到 backup@example.com/ })).toHaveAttribute(
      "title",
      "当前正在使用这个账号",
    );
  });

  it("does not show an empty quota placeholder for enterprise accounts without quota windows", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: [
          {
            name: "enterprise@example.com",
            codexHome: "/Users/alex/.modex/111111111111",
            authType: "chatGpt",
            apiBaseUrl: null,
            loggedIn: true,
            loginExpired: false,
            isCurrent: false,
            quota: {
              status: "available",
              plan: "企业版",
              primaryLabel: "5小时已用 -",
              primaryPercent: 0,
              primaryResetAt: null,
              secondaryLabel: "每周已用 -",
              secondaryPercent: 0,
              secondaryResetAt: null,
              credits: "额度可用",
            },
          },
        ],
        currentIdentityName: null,
      }),
    );

    render(<App />);

    const row = await screen.findByRole("article", { name: /enterprise@example.com/ });
    expect(within(row).queryByText("暂无可展示的额度信息")).not.toBeInTheDocument();
    expect(within(row).queryByText(/5小时已用/)).not.toBeInTheDocument();
    expect(within(row).queryByText(/本周已用/)).not.toBeInTheDocument();
    expect(row.querySelector(".quota-meter")).not.toBeInTheDocument();
  });

  it("does not show action success messages under the title", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: state().identities.map((identity) =>
          identity.name === "backup@example.com" ? { ...identity, loggedIn: true, loginExpired: false } : identity,
        ),
      }),
    );
    mockApi.switchIdentity.mockResolvedValue({ ok: true, message: "正在切换到账号：backup@example.com" });

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /切换到 backup@example.com/ }));

    await waitFor(() => expect(mockApi.switchIdentity).toHaveBeenCalledWith("backup@example.com"));
    expect(screen.queryByText("正在切换到账号：backup@example.com")).not.toBeInTheDocument();
  });

  it("shows switch loading before invoking the backend switch", async () => {
    let dialogWasVisibleWhenSwitchStarted = false;
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: state().identities.map((identity) =>
          identity.name === "backup@example.com" ? { ...identity, loggedIn: true, loginExpired: false } : identity,
        ),
      }),
    );
    mockApi.switchIdentity.mockImplementation(() => {
      dialogWasVisibleWhenSwitchStarted = Boolean(screen.queryByRole("dialog", { name: "处理中" }));
      return new Promise(() => undefined);
    });

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /切换到 backup@example.com/ }));

    await waitFor(() => expect(mockApi.switchIdentity).toHaveBeenCalledWith("backup@example.com"));
    expect(dialogWasVisibleWhenSwitchStarted).toBe(true);
    expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();
    expect(screen.queryByText("正在切换账号")).not.toBeInTheDocument();
  });

  it("does not show an in-app error when account switching fails", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: state().identities.map((identity) =>
          identity.name === "backup@example.com" ? { ...identity, loggedIn: true, loginExpired: false } : identity,
        ),
      }),
    );
    mockApi.switchIdentity.mockRejectedValue(new Error("Codex 未退出，账号切换已取消。"));

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /切换到 backup@example.com/ }));

    await waitFor(() => expect(mockApi.switchIdentity).toHaveBeenCalledWith("backup@example.com"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "处理中" })).not.toBeInTheDocument());
    expect(screen.queryByText("Codex 未退出，账号切换已取消。")).not.toBeInTheDocument();
    expect(screen.getByRole("article", { name: /team@example.com/ })).toHaveClass("current");
    expect(screen.getByRole("article", { name: /backup@example.com/ })).not.toHaveClass("current");
  });

  it("routes action failures into the collapsed log panel with an unread dot", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: state().identities.map((identity) =>
          identity.name === "backup@example.com" ? { ...identity, loggedIn: true, loginExpired: false } : identity,
        ),
      }),
    );
    mockApi.switchIdentity.mockRejectedValue(new Error("Codex 未退出，账号切换已取消。"));

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /切换到 backup@example.com/ }));

    await waitFor(() => expect(mockApi.switchIdentity).toHaveBeenCalledWith("backup@example.com"));
    expect(screen.queryByText("Codex 未退出，账号切换已取消。")).not.toBeInTheDocument();

    const logButton = screen.getByRole("button", { name: "打开日志" });
    expect(logButton.querySelector(".unread-dot")).toBeInTheDocument();

    await userEvent.click(logButton);

    const panel = await screen.findByRole("region", { name: "日志" });
    expect(within(panel).getByText("操作失败")).toBeInTheDocument();
    expect(within(panel).getByText(/Codex 未退出/)).toBeInTheDocument();
    expect(logButton.querySelector(".unread-dot")).not.toBeInTheDocument();
  });

  it("keeps the log panel main-screen only and lets the toolbar button toggle it", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    const logButton = screen.getByRole("button", { name: "打开日志" });
    await userEvent.click(logButton);

    expect(await screen.findByRole("region", { name: "日志" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "关闭日志" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "关闭日志" }));

    expect(screen.queryByRole("region", { name: "日志" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "打开日志" }));
    expect(await screen.findByRole("region", { name: "日志" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));

    expect(screen.queryByRole("region", { name: "日志" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "打开日志" })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "返回账号" }));
    expect(screen.getByRole("button", { name: "关闭日志" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "日志" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "关闭日志面板" }));

    expect(screen.queryByRole("region", { name: "日志" })).not.toBeInTheDocument();
  });

  it("renders a newly added account and unlocks row actions while browser login is still pending", async () => {
    const pendingLogin = new Promise(() => undefined);
    const pendingIdentity = {
      name: "登录中",
      codexHome: "/Users/alex/.modex/333333333333",
      authType: "chatGpt" as const,
      apiBaseUrl: null,
      loggedIn: false,
      loginExpired: false,
      isCurrent: false,
      quota: {
        status: "unknown" as const,
        plan: "计划未知",
        primaryLabel: "5小时已用 -",
        primaryPercent: 0,
        primaryResetAt: null,
        secondaryLabel: "每周已用 -",
        secondaryPercent: 0,
        secondaryResetAt: null,
        credits: "额度未知",
      },
    };
    mockApi.getAppState
      .mockResolvedValueOnce(state())
      .mockResolvedValueOnce(state({ identities: [...state().identities, pendingIdentity] }));
    mockApi.addIdentity.mockResolvedValue(pendingIdentity);
    mockApi.loginIdentity.mockReturnValue(pendingLogin);

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: /新增账号/ }));
    await userEvent.click(await screen.findByRole("button", { name: "用浏览器登录" }));

    await waitFor(() => expect(mockApi.loginIdentity).toHaveBeenCalledWith("登录中"));
    const pendingRow = await screen.findByRole("article", { name: /登录中/ });

    expect(mockApi.getAppState).toHaveBeenCalledTimes(2);
    expect(within(pendingRow).getByRole("button", { name: /删除 登录中/ })).not.toBeDisabled();
    expect(screen.queryByText(/已打开浏览器登录/)).not.toBeInTheDocument();
  });

  it("automatically imports the current Codex account on startup and refreshes it", async () => {
    const importedIdentity = {
      name: "imported@example.com · 团队版",
      codexHome: "/Users/alex/.modex/333333333333",
      authType: "chatGpt" as const,
      apiBaseUrl: null,
      loggedIn: true,
      loginExpired: false,
      isCurrent: false,
      quota: {
        status: "unknown" as const,
        plan: "团队版",
        primaryLabel: "5小时已用 -",
        primaryPercent: 0,
        primaryResetAt: null,
        secondaryLabel: "每周已用 -",
        secondaryPercent: 0,
        secondaryResetAt: null,
        credits: "额度未知",
      },
    };
    mockApi.getAppState
      .mockResolvedValueOnce(state())
      .mockResolvedValue(state({ identities: [...state().identities, importedIdentity] }));
    mockApi.importCurrentIdentity.mockResolvedValue({
      ok: true,
      message: "已导入账号：imported@example.com · 团队版",
      identity: importedIdentity,
      imported: true,
    });
    mockApi.refreshIdentity.mockResolvedValue(importedIdentity);

    render(<App />);

    await waitFor(() => expect(mockApi.importCurrentIdentity).toHaveBeenCalledTimes(1));
    expect(mockApi.refreshIdentity).toHaveBeenCalledWith("imported@example.com · 团队版");
    expect(await screen.findByRole("article", { name: /imported@example.com/ })).toBeInTheDocument();
    expect(screen.queryByText(/已导入账号/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /导入当前账号/ })).not.toBeInTheDocument();
  });

  it("reloads state when automatic import reuses an existing account as current", async () => {
    const initial = state({ currentIdentityName: "team@example.com" });
    const reusedIdentity = { ...initial.identities[1], loggedIn: true, loginExpired: false, isCurrent: true };
    mockApi.getAppState
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(
        state({
          currentIdentityName: "backup@example.com",
          identities: initial.identities.map((identity) =>
            identity.name === "backup@example.com" ? reusedIdentity : { ...identity, isCurrent: false },
          ),
        }),
      );
    mockApi.importCurrentIdentity.mockResolvedValue({
      ok: true,
      message: "账号已存在，未重复导入：backup@example.com",
      identity: reusedIdentity,
      imported: false,
    });

    render(<App />);

    await waitFor(() => expect(mockApi.importCurrentIdentity).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockApi.getAppState).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("article", { name: /backup@example.com/ })).toHaveClass("current");
    expect(mockApi.refreshIdentity).not.toHaveBeenCalled();
  });

  it("does not show an error banner when automatic import finds no source login", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.importCurrentIdentity.mockResolvedValue({
      ok: false,
      message: "当前 Codex 尚未登录，无法导入。",
      identity: null,
      imported: false,
    });

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });

    await waitFor(() => expect(mockApi.importCurrentIdentity).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("当前 Codex 尚未登录，无法导入。")).not.toBeInTheDocument();
    expect(mockApi.refreshIdentity).not.toHaveBeenCalled();
  });

  it("refreshes quota for the matching account after browser login succeeds", async () => {
    let finishRefresh: (value: unknown) => void = () => undefined;
    const pendingIdentity = {
      name: "登录中",
      codexHome: "/Users/alex/.modex/333333333333",
      authType: "chatGpt" as const,
      apiBaseUrl: null,
      loggedIn: false,
      loginExpired: false,
      isCurrent: false,
      quota: {
        status: "unknown" as const,
        plan: "计划未知",
        primaryLabel: "5小时已用 -",
        primaryPercent: 0,
        primaryResetAt: null,
        secondaryLabel: "每周已用 -",
        secondaryPercent: 0,
        secondaryResetAt: null,
        credits: "额度未知",
      },
    };
    const loggedInIdentity = {
      ...pendingIdentity,
      name: "new@example.com",
      loggedIn: true,
      quota: {
        ...pendingIdentity.quota,
        plan: "团队版",
      },
    };
    mockApi.getAppState
      .mockResolvedValueOnce(state())
      .mockResolvedValueOnce(state({ identities: [...state().identities, pendingIdentity] }))
      .mockResolvedValueOnce(state({ identities: [...state().identities, loggedInIdentity] }))
      .mockResolvedValue(state({ identities: [...state().identities, loggedInIdentity] }));
    mockApi.addIdentity.mockResolvedValue(pendingIdentity);
    mockApi.loginIdentity.mockResolvedValue({ ok: true, message: "已打开浏览器登录：登录中" });
    mockApi.refreshIdentity.mockImplementation(
      () =>
        new Promise((resolve) => {
          finishRefresh = resolve;
        }),
    );

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    const realSetTimeout = window.setTimeout.bind(window);
    const timeoutSpy = vi.spyOn(window, "setTimeout").mockImplementation(((handler: TimerHandler, timeout?: number, ...args: unknown[]) =>
      realSetTimeout(handler, timeout === 2000 ? 0 : timeout, ...args)) as typeof window.setTimeout);
    try {
      await userEvent.click(screen.getByRole("button", { name: /新增账号/ }));
      await userEvent.click(await screen.findByRole("button", { name: "用浏览器登录" }));

      await waitFor(() => expect(mockApi.loginIdentity).toHaveBeenCalledWith("登录中"));
      await waitFor(() => expect(mockApi.refreshIdentity).toHaveBeenCalledWith("new@example.com"));
      expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();

      finishRefresh(loggedInIdentity);

      await waitFor(() => expect(mockApi.getAppState).toHaveBeenCalledTimes(4));
    } finally {
      timeoutSpy.mockRestore();
    }
  });

  it("opens the account directory without showing a success banner or reloading accounts", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.openIdentityDirectory.mockResolvedValue({ ok: true, message: "已打开账号目录" });

    render(<App />);

    const teamRow = await screen.findByRole("article", { name: /team@example.com/ });
    await userEvent.click(within(teamRow).getByRole("button", { name: /打开 team@example.com 配置目录/ }));

    await waitFor(() => expect(mockApi.openIdentityDirectory).toHaveBeenCalledWith("team@example.com"));
    expect(screen.queryByText("已打开账号目录")).not.toBeInTheDocument();
    expect(mockApi.getAppState).toHaveBeenCalledTimes(1);
    expect(mockApi.refreshAll).not.toHaveBeenCalled();
  });

  it("confirms account deletion by removing only the deleted row locally", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.deleteIdentity.mockResolvedValue({ ok: true, message: "已删除账号" });

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /删除 backup@example.com/ }));

    const dialog = await screen.findByRole("dialog", { name: "删除账号" });
    expect(within(dialog).getByText("确定要删除账号 backup@example.com 吗？")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认删除" }));

    await waitFor(() => expect(mockApi.deleteIdentity).toHaveBeenCalledWith("backup@example.com"));
    expect(screen.queryByText("已删除账号")).not.toBeInTheDocument();
    expect(screen.queryByRole("article", { name: /backup@example.com/ })).not.toBeInTheDocument();
    expect(mockApi.getAppState).toHaveBeenCalledTimes(1);
    expect(mockApi.refreshAll).not.toHaveBeenCalled();
  });

  it("does not delete an account when deletion confirmation is cancelled", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    const backupRow = await screen.findByRole("article", { name: /backup@example.com/ });
    await userEvent.click(within(backupRow).getByRole("button", { name: /删除 backup@example.com/ }));
    const dialog = await screen.findByRole("dialog", { name: "删除账号" });
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));

    expect(mockApi.deleteIdentity).not.toHaveBeenCalled();
    expect(mockApi.refreshAll).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: "删除账号" })).not.toBeInTheDocument();
  });

  it("does not allow switching to an account without a local login", async () => {
    mockApi.getAppState.mockResolvedValue(
      state({
        identities: [
          ...state().identities,
          {
            name: "not-logged-in@example.com",
            codexHome: "/Users/alex/.modex/222222222222",
            authType: "chatGpt",
            apiBaseUrl: null,
            loggedIn: false,
            loginExpired: false,
            isCurrent: false,
            quota: {
              status: "unknown",
              plan: "计划未知",
              primaryLabel: "5小时已用 -",
              primaryPercent: 0,
              primaryResetAt: null,
              secondaryLabel: "每周已用 -",
              secondaryPercent: 0,
              secondaryResetAt: null,
              credits: "额度未知",
            },
          },
        ],
      }),
    );

    render(<App />);

    const row = await screen.findByRole("article", { name: /not-logged-in@example.com/ });

    expect(within(row).getByRole("button", { name: /切换到 not-logged-in@example.com/ })).toBeDisabled();
  });

  it("shows global refresh loading and reloads state when refresh completes", async () => {
    let finishRefresh: (value: unknown) => void = () => undefined;
    let dialogWasVisibleWhenRefreshStarted = false;
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.refreshAll.mockImplementation(
      () => {
        dialogWasVisibleWhenRefreshStarted = Boolean(screen.queryByRole("dialog", { name: "处理中" }));
        return new Promise((resolve) => {
          finishRefresh = resolve;
        });
      },
    );

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    const refreshButton = screen.getByRole("button", { name: /刷新全部账号/ });
    expect(refreshButton).toHaveAttribute("aria-busy", "false");

    await userEvent.click(refreshButton);

    await waitFor(() => expect(mockApi.refreshAll).toHaveBeenCalledTimes(1));
    expect(dialogWasVisibleWhenRefreshStarted).toBe(true);
    expect(refreshButton).toBeDisabled();
    expect(refreshButton).toHaveAttribute("aria-busy", "true");
    expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();
    expect(screen.queryByText("正在刷新账号信息")).not.toBeInTheDocument();
    expect(screen.queryByText("Modex 正在同步账号状态，主界面会保持可见。")).not.toBeInTheDocument();
    expect(screen.queryByText("正在刷新")).not.toBeInTheDocument();

    finishRefresh([]);

    await waitFor(() => expect(mockApi.getAppState).toHaveBeenCalledTimes(2));
  });

  it("shows the refresh dialog while backend refresh events are active", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    expect(eventMocks.listen).toHaveBeenCalledWith("modex://refresh-started", expect.any(Function));
    expect(eventMocks.listen).toHaveBeenCalledWith("modex://refresh-finished", expect.any(Function));

    eventMocks.listeners.get("modex://refresh-started")?.();

    expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "刷新全部账号" })).not.toBeDisabled();

    eventMocks.listeners.get("modex://refresh-finished")?.();

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "处理中" })).not.toBeInTheDocument());
  });

  it("shows the refresh dialog during startup refresh before state loads", async () => {
    mockApi.getAppState.mockImplementation(() => new Promise(() => undefined));

    render(<App />);

    await waitFor(() => expect(eventMocks.listen).toHaveBeenCalledWith("modex://refresh-started", expect.any(Function)));
    eventMocks.listeners.get("modex://refresh-started")?.();

    expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();
    expect(screen.getByText("正在加载 Modex")).toBeInTheDocument();
  });

  it("shows the refresh dialog when initial app state is already refreshing", async () => {
    mockApi.getAppState.mockResolvedValue(state({ isRefreshing: true }));

    render(<App />);

    expect(await screen.findByRole("dialog", { name: "处理中" })).toBeInTheDocument();
  });

  it("reloads app state when the backend emits state-updated", async () => {
    mockApi.getAppState
      .mockResolvedValueOnce(state())
      .mockResolvedValueOnce(state({ currentIdentityName: "backup@example.com" }));

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    expect(eventMocks.listen).toHaveBeenCalledWith("modex://state-updated", expect.any(Function));

    eventMocks.listeners.get("modex://state-updated")?.();

    await waitFor(() => expect(mockApi.getAppState).toHaveBeenCalledTimes(2));
  });

  it("renders settings as a full-width form without header copy or a settings toggle", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));

    expect(screen.queryByText("Global configuration")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "全局设置" })).not.toBeInTheDocument();
    expect(screen.queryByText("这些设置会影响 Modex 全局行为，不会只作用于当前账号。")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Account workbench" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /刷新全部账号/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "关闭全局设置" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "返回账号" })).not.toHaveTextContent("返回账号");

    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    expect(styles).toMatch(/\.settings-panel\s*{[^}]*width:\s*100%/s);
    expect(styles).toMatch(/\.settings-panel\s*{[^}]*max-width:\s*none/s);
  });

  it("uses a stronger brand title treatment with lighter borders and quota meters", () => {
    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

    expect(styles).toMatch(/\.brand-mark\s*{/);
    expect(styles).toMatch(/\.brand-word\s*{[^}]*font-weight:\s*900/s);
    expect(styles).toMatch(/\.meter-track\s*{[^}]*height:\s*6px/s);
    expect(styles).toMatch(/\.meter-fill\s*{[^}]*opacity:\s*0\.42/s);
    expect(styles).toMatch(/\.quota-percent\s*{[^}]*font-weight:\s*900/s);
    expect(styles).toMatch(/\.toolbar\s*{(?![^}]*border:)[^}]*}/s);
    expect(styles).toMatch(/\.account-card,\s*\.settings-panel,\s*\.empty-state\s*{(?![^}]*border:)[^}]*}/s);
  });

  it("uses compact quota meters with usage-based colors", () => {
    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

    expect(styles).toMatch(/grid-template-columns:\s*400px minmax\(210px, 0\.56fr\) auto/);
    expect(styles).toMatch(/\.account-card\s*{[^}]*gap:\s*6px/s);
    expect(styles).toMatch(/\.account-title-line\s*{[^}]*flex-wrap:\s*nowrap/s);
    expect(styles).toMatch(/\.account-status-slot\s*{[^}]*flex:\s*0 0 92px/s);
    expect(styles).toMatch(/\.account-status-slot\s*{[^}]*justify-content:\s*flex-start/s);
    expect(styles).not.toMatch(/\.account-status\s*{[^}]*width:/s);
    expect(styles).toMatch(/\.meter-fill\.good\s*{[^}]*background:\s*var\(--success\)/s);
    expect(styles).toMatch(/\.meter-fill\.warn\s*{[^}]*background:\s*var\(--limited\)/s);
    expect(styles).toMatch(/\.meter-fill\.danger\s*{[^}]*background:\s*var\(--danger\)/s);
  });

  it("keeps account rows at a fixed minimum size and scrolls the list when constrained", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    expect(await screen.findByRole("region", { name: "账号列表" })).toBeInTheDocument();

    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    expect(styles).toMatch(/\.account-list\s*{[^}]*overflow:\s*auto/s);
    expect(styles).toMatch(/\.account-card\s*{[^}]*min-width:\s*900px/s);
    expect(styles).not.toMatch(/@media\s*\(max-width:\s*1020px\)[\s\S]*?\.account-card\s*{[\s\S]*?grid-template-columns:\s*1fr/s);
  });

  it("uses flat light and dark mode tokens without the old gray list tail", () => {
    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

    expect(styles).toMatch(/color-scheme:\s*light dark/);
    expect(styles).toMatch(/@media\s*\(prefers-color-scheme:\s*dark\)/);
    expect(styles).toMatch(/\.account-list\s*{[^}]*padding:\s*0/s);
    expect(styles).toMatch(/\.refresh-dialog\s*{[^}]*top:\s*50%/s);
    expect(styles).toMatch(/\.refresh-dialog\s*{[^}]*transform:\s*translate\(-50%,\s*-50%\)/s);
    expect(styles).not.toMatch(/box-shadow:\s*0\s+[1-9]/);
  });

  it("does not overwrite unsaved settings edits when quota state updates arrive", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    const codexInput = screen.getByLabelText("Codex CLI");
    await userEvent.clear(codexInput);
    await userEvent.type(codexInput, "/custom/codex");

    eventMocks.listeners.get("modex://state-updated")?.();

    await waitFor(() => expect(mockApi.getAppState).toHaveBeenCalledTimes(2));
    expect(screen.getByLabelText("Codex CLI")).toHaveValue("/custom/codex");
  });

  it("saves settings from the settings view", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateSettings.mockResolvedValue(state({ codexBinary: "/opt/codex", pollSeconds: 90 }));

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    await userEvent.clear(screen.getByLabelText("Codex CLI"));
    await userEvent.type(screen.getByLabelText("Codex CLI"), "/opt/codex");
    await userEvent.clear(screen.getByLabelText("刷新间隔"));
    await userEvent.type(screen.getByLabelText("刷新间隔"), "90");
    await userEvent.click(screen.getByRole("button", { name: "保存全局设置" }));

    await waitFor(() => {
      expect(mockApi.updateSettings).toHaveBeenCalledWith({
        codexBinary: "/opt/codex",
        appName: "Codex",
        sourceHome: "/Users/alex/.codex",
        pollSeconds: 90,
      });
    });
    expect(await screen.findByRole("status")).toHaveTextContent("全局设置已保存");
  });

  it("saves settings without disabling the settings UI or reloading state", async () => {
    let resolveSettings!: (settings: AppSettings) => void;
    const saveSettings = new Promise<AppSettings>((resolve) => {
      resolveSettings = resolve;
    });
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateSettings.mockReturnValue(saveSettings);

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    await userEvent.clear(screen.getByLabelText("Codex CLI"));
    await userEvent.type(screen.getByLabelText("Codex CLI"), "/quiet/codex");
    await userEvent.click(screen.getByRole("button", { name: "保存全局设置" }));

    await waitFor(() => expect(mockApi.updateSettings).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "保存全局设置" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "保存自动唤醒" })).not.toBeDisabled();
    expect(screen.getByLabelText("Codex CLI")).toHaveValue("/quiet/codex");
    expect(mockApi.getAppState).toHaveBeenCalledTimes(1);

    resolveSettings(state({ codexBinary: "/quiet/codex" }));

    expect(await screen.findByRole("status")).toHaveTextContent("全局设置已保存");
    expect(mockApi.getAppState).toHaveBeenCalledTimes(1);
  });

  it("renders and saves daily wake settings", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateDailyWakeSettings.mockResolvedValue(
      state({
        dailyWake: {
          ...state().dailyWake,
          enabled: true,
          time: "09:15",
          times: ["09:15"],
          skipIfPrimaryUsedAbovePercent: 3,
          skipIfWeeklyRemainingBelowPercent: 20,
        },
      }),
    );

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    const wakeSwitch = screen.getByRole("switch", { name: "每日自动唤醒" });
    expect(wakeSwitch).toHaveAttribute("aria-checked", "false");
    await userEvent.click(wakeSwitch);
    expect(wakeSwitch).toHaveAttribute("aria-checked", "true");
    await userEvent.clear(screen.getByLabelText("执行时间 1"));
    await userEvent.type(screen.getByLabelText("执行时间 1"), "09:15");
    await userEvent.clear(screen.getByLabelText("5 小时用量超过"));
    await userEvent.type(screen.getByLabelText("5 小时用量超过"), "3");
    await userEvent.clear(screen.getByLabelText("本周剩余额度低于"));
    await userEvent.type(screen.getByLabelText("本周剩余额度低于"), "20");
    await userEvent.click(screen.getByRole("button", { name: "保存自动唤醒" }));

    await waitFor(() =>
      expect(mockApi.updateDailyWakeSettings).toHaveBeenCalledWith({
        enabled: true,
        time: "09:15",
        times: ["09:15"],
        message: "Good morning",
        skipIfPrimaryUsedAbovePercent: 3,
        skipIfWeeklyRemainingBelowPercent: 20,
        maxPrimaryDeltaPercent: 3,
        lastRunDate: null,
        lastRunSlots: [],
      }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent("唤醒设置已保存");
  });

  it("adds and saves multiple daily wake times", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateDailyWakeSettings.mockResolvedValue(
      state({
        dailyWake: {
          ...state().dailyWake,
          times: ["08:30", "14:00"],
        },
      }),
    );

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    await userEvent.click(screen.getByRole("button", { name: "新增时间" }));

    const timeInputs = [screen.getByLabelText("执行时间 1"), screen.getByLabelText("执行时间 2")];
    expect(timeInputs).toHaveLength(2);
    await userEvent.clear(timeInputs[1]);
    await userEvent.type(timeInputs[1], "14:00");
    await userEvent.click(screen.getByRole("button", { name: "保存自动唤醒" }));

    await waitFor(() =>
      expect(mockApi.updateDailyWakeSettings).toHaveBeenCalledWith({
        enabled: false,
        time: "08:30",
        times: ["08:30", "14:00"],
        message: "Good morning",
        skipIfPrimaryUsedAbovePercent: 3,
        skipIfWeeklyRemainingBelowPercent: 20,
        maxPrimaryDeltaPercent: 3,
        lastRunDate: null,
        lastRunSlots: [],
      }),
    );
  });

  it("runs an immediate daily wake test from settings", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateDailyWakeSettings.mockResolvedValue(
      state({
        dailyWake: {
          ...state().dailyWake,
          message: "Wake test",
        },
      }),
    );
    mockApi.runDailyWakeNow.mockResolvedValue({
      ok: true,
      message: "已完成一次测试唤醒，详情见日志。",
    });

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    await userEvent.clear(screen.getByLabelText("唤醒提示语"));
    await userEvent.type(screen.getByLabelText("唤醒提示语"), "Wake test");
    await userEvent.click(screen.getByRole("button", { name: "立即测试" }));

    await waitFor(() =>
      expect(mockApi.updateDailyWakeSettings).toHaveBeenCalledWith({
        enabled: false,
        time: "08:30",
        times: ["08:30"],
        message: "Wake test",
        skipIfPrimaryUsedAbovePercent: 3,
        skipIfWeeklyRemainingBelowPercent: 20,
        maxPrimaryDeltaPercent: 3,
        lastRunDate: null,
        lastRunSlots: [],
      }),
    );
    await waitFor(() => expect(mockApi.runDailyWakeNow).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("status")).toHaveTextContent("测试唤醒已完成");
  });

  it("shows failure notices when settings saves fail", async () => {
    mockApi.getAppState.mockResolvedValue(state());
    mockApi.updateSettings.mockRejectedValueOnce(new Error("config is locked"));
    mockApi.updateDailyWakeSettings.mockRejectedValueOnce(new Error("invalid wake time"));

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));
    await userEvent.click(screen.getByRole("button", { name: "保存全局设置" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("全局设置保存失败");
    expect(screen.getByRole("alert")).toHaveTextContent("config is locked");

    await userEvent.click(screen.getByRole("button", { name: "保存自动唤醒" }));

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("唤醒设置保存失败"));
    expect(screen.getByRole("alert")).toHaveTextContent("invalid wake time");
  });

  it("explains wake thresholds with hover help icons", async () => {
    mockApi.getAppState.mockResolvedValue(state());

    render(<App />);

    await screen.findByRole("heading", { name: "Modex", level: 1 });
    await userEvent.click(screen.getByRole("button", { name: "打开全局设置" }));

    expect(screen.getByRole("button", { name: "说明：5 小时用量超过" })).toHaveAttribute(
      "data-tooltip",
      expect.stringContaining("5 小时窗口用量超过该比例时跳过唤醒"),
    );
    expect(screen.getByRole("button", { name: "说明：本周剩余额度低于" })).toHaveAttribute(
      "data-tooltip",
      expect.stringContaining("本周剩余额度低于该比例时跳过唤醒"),
    );
    expect(screen.getByRole("button", { name: "说明：用量异常增长超过" })).toHaveAttribute(
      "data-tooltip",
      expect.stringContaining("唤醒后 5 小时用量增长超过该比例"),
    );

    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");
    expect(styles).toMatch(/\.help-tip:hover::after/s);
    expect(styles).toMatch(/\.field-label-row\s*{[^}]*display:\s*flex/s);
  });

  it("does not define CSS gradients", () => {
    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

    expect(styles).not.toMatch(/gradient/i);
  });

  it("uses a floating right-side log panel and scrollable settings content", () => {
    const styles = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

    expect(styles).toMatch(/\.workspace\s*{[^}]*position:\s*relative/s);
    expect(styles).toMatch(/\.log-panel\s*{[^}]*position:\s*absolute/s);
    expect(styles).toMatch(/\.log-panel\s*{[^}]*right:\s*20px/s);
    expect(styles).toMatch(/\.log-panel\s*{[^}]*bottom:\s*20px/s);
    expect(styles).toMatch(/\.log-panel\s*{[^}]*border-left:\s*1px solid/s);
    expect(styles).toMatch(/\.log-entry::before\s*{[^}]*border-radius:\s*999px/s);
    expect(styles).toMatch(/\.switch-control\s*{[^}]*border-radius:\s*999px/s);
    expect(styles).toMatch(/\.settings-content\s*{[^}]*overflow:\s*auto/s);
  });
});
