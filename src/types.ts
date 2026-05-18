export type QuotaStatus = "unknown" | "available" | "limited" | "error";

export type QuotaDisplay = {
  status: QuotaStatus;
  plan: string;
  primaryLabel: string;
  primaryPercent: number;
  primaryResetAt?: number | null;
  secondaryLabel: string;
  secondaryPercent: number;
  secondaryResetAt?: number | null;
  credits: string;
  error?: string | null;
};

export type Identity = {
  name: string;
  codexHome: string;
  workspaceId?: string | null;
  loggedIn: boolean;
  loginExpired: boolean;
  isCurrent: boolean;
  quota: QuotaDisplay;
};

export type AppSettings = {
  codexBinary: string;
  appName: string;
  pollSeconds: number;
  sourceHome: string;
  hasCompletedSetup: boolean;
  currentIdentityName?: string | null;
  isRefreshing: boolean;
  identities: Identity[];
};

export type SettingsPatch = Partial<Pick<AppSettings, "codexBinary" | "appName" | "pollSeconds" | "sourceHome">>;

export type ActionResult = {
  ok: boolean;
  message: string;
};

export type ImportIdentityResult = ActionResult & {
  identity?: Identity | null;
  imported: boolean;
};
