import { invoke } from "@tauri-apps/api/core";

// ============================================================
// 后端 IPC 类型化封装 —— 对应 src-tauri 的 #[tauri::command]。
// 契约见 docs/gui-rebuild-tauri.md §5。
// 注意：Config 内部字段是 serde 原名（snake_case，兼容红线 #2）。
// ============================================================

/** 单个模型的费率（§3.1），单位「每百万 token」。四个字段全部可选。 */
export type ModelPricing = {
  input?: number;
  output?: number;
  cache_read?: number;
  cache_write?: number;
};

export type ModelEntry = {
  name: string;
  /** 非空（v1 里为 "auto"）表示该模型开启 1M 上下文变体。 */
  to_1m: string;
  /** 从 models.dev 同步来的费率（USD/百万 token），后端专管，界面只读。 */
  pricing_synced?: ModelPricing;
  /** 上游该模型的最大上下文（token），由 models.dev 同步填，界面只读。 */
  context_limit?: number;
  /** 它在 Claude 里用的名字（claude-opus-5 这类），决定有没有 Auto 模式、能调几档思考。没名字的行没有。 */
  slot?: string;
};

export type Provider = {
  target_url: string;
  api_key: string;
  models: ModelEntry[];
  /** "" 默认（不干预）/ "off" 关闭思考 / "high" 标准 / "max" 深度 */
  thinking_effort: string;
};

export type Config = {
  providers: Provider[];
  /** 上次成功应用的配置摘要（应用状态机 dirty 判定，后端专管）。 */
  last_applied_hash?: string;
  /** 上次应用时间（Unix 秒字符串）。 */
  last_applied_at?: string;
  /** 代理端口（后端专管，set_port 热切换；缺省 5678）。 */
  port?: number;
  /** 上次「应用」时用的槽位池代号；与当前不符 = 升级后还没重新应用。 */
  last_applied_pool?: string;
  /** 上次「应用」时代理的端口（后端专管）；没有 = 2.2 之前应用的或从没应用过。 */
  last_applied_port?: number;
  /** 启动时自动从 models.dev 同步费率（6 小时阈值），默认开。 */
  pricing_auto_sync?: boolean;
  /** 上次成功同步时间（Unix 秒字符串）。 */
  pricing_synced_at?: string;
};

/** 服务商在 models.dev 上列出的一个模型。 */
export type AvailableModel = { id: string; context: number | null };

/** 检测到的 Claude Desktop 版本 + 因版本过低不可用的键（§3.8）。 */
export type DesktopInfo = { version: string | null; unavailable: string[] };

export type PricingSyncResult = {
  ok: boolean;
  changed: number;
  skipped: boolean;
  message: string;
  synced_at: string;
};

export type ProxyStatus = { running: boolean; port: number };

/** Claude Desktop 实际在用的网关配置里的一个模型条目。 */
export type AppliedModel = {
  slot: string;
  /** labelOverride；老版本写入的条目没有，为空 */
  label: string;
  supports_1m: boolean;
};

/** Claude Desktop 实际在用的网关配置（只读读回）。 */
export type AppliedState = {
  /** 找到并读懂了那份配置文件 */
  found: boolean;
  provider: string;
  gateway_url: string;
  models: AppliedModel[];
};

/**
 * 「要不要应用」里逐槽位比对管不到的部分（后端按 Claude 实际写着的配置判断）。
 * 只改密钥 / 地址 / 默认档时这几项都不会变 —— 代理当场就用上了，不必重启 Claude。
 */
export type PendingApply = {
  /** 找到并读懂了 Claude 正在用的那份配置 */
  found: boolean;
  /** 网关没指向这个代理（没接到 ModelLink，或端口对不上） */
  gateway: boolean;
  /** 端口在上次应用之后换过（Claude 要重启才连得上新端口）；null = 不知道 */
  port_changed: boolean | null;
  /** 写进 Claude 的费率表过期了 */
  pricing: boolean;
  /** Claude 里「允许联网的域名」不再放行全部：Cowork 和 Code 里抓网页、装包会失败 */
  egress: boolean;
  /** Claude 里的模型身份说明没了：模型可能自称 Claude */
  identity: boolean;
};

/** 一键配置里的一项（设置页「一键使用 Winhao 的配置」）。 */
export type PresetItem = {
  /** 写进 Claude 配置文件的键名 */
  key: string;
  want: boolean;
  /** Claude 眼下实际生效的值（没写就是 Claude 的默认值） */
  current: boolean;
  /** 装的 Claude Desktop 版本认不认这个键；不认的应用时跳过 */
  supported: boolean;
};

export type PresetState = {
  /** 找到了 Claude 正在用的配置文件（没有就得先应用一次） */
  found: boolean;
  items: PresetItem[];
};

/** 上游 usage 里的 token 数。 */
export type Usage = {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
};

export type LogEntry = {
  id: number;
  time: string;
  /** Claude 请求的槽位名（原样，可能带 [1m]）。 */
  slot: string;
  /** 实际发给上游的模型名；未映射时就是请求的槽位名。 */
  model: string;
  status: number;
  /** 实际发给上游的推理强度（""=未发 / "off" / low…max）。 */
  thinking: string;
  /** 附注：整流标记 / 未映射槽位等，多条以「；」连接（标记文字里自带「 · 」）。 */
  note: string;
  /** ModelLink 自己判定为错误的请求（标红）。 */
  error: boolean;
  /** 耗时（毫秒）；null = 还在传。 */
  duration_ms: number | null;
  /** 上游没给、或响应中途断开时为 null。 */
  usage: Usage | null;
  /** 按这个模型的费率算出的花费（USD）；没费率或没用量时为 null —— 绝不估算。 */
  cost_usd: number | null;
  /** 出错时上游给的说明。 */
  detail: string;
};

/** 今天的请求统计（不受「只留 100 条」限制）。 */
export type TodayStats = {
  /** 从这个时刻起算（Unix 秒）：本地零点，或 ModelLink 今天启动的时刻 */
  since: number;
  requests: number;
  failures: number;
  duration_total_ms: number;
  duration_max_ms: number;
  /** 输入侧合计（含缓存） */
  input_tokens: number;
  output_tokens: number;
  /** 拿到用量的请求数 */
  with_usage: number;
  cost_usd: number;
  /** 算得出花费的请求数；少于 with_usage 说明有模型没费率 */
  priced: number;
};

export type TestResult = { ok: boolean; message: string };

export const guiVersion = () => invoke<string>("gui_version");
export const getConfig = () => invoke<Config>("get_config");
/** 返回后端合并「后端专管」字段后的那份配置 —— 前端应据它算 dirty。 */
export const saveConfig = (config: Config) => invoke<Config>("save_config", { config });
export const configHash = (config: Config) => invoke<string>("config_hash", { config });
export const testProvider = (targetUrl: string, apiKey: string, model: string) =>
  invoke<TestResult>("test_provider", { targetUrl, apiKey, model });
export const applyToClaude = () => invoke<string>("apply_to_claude");
export const getLogs = () => invoke<LogEntry[]>("get_logs");
export const getLogStats = () => invoke<TodayStats>("get_log_stats");
export const proxyStatus = () => invoke<ProxyStatus>("proxy_status");
export const setPort = (port: number) => invoke<ProxyStatus>("set_port", { port });
export const syncPricing = (force: boolean) =>
  invoke<PricingSyncResult>("sync_pricing", { force });
export const desktopInfo = () => invoke<DesktopInfo>("desktop_info");
/** 读回 Claude Desktop 眼下实际在用的网关配置（概览页逐槽位「已生效 / 未应用」的依据）。 */
export const appliedState = () => invoke<AppliedState>("applied_state");
/** 见 {@link PendingApply}。 */
export const pendingApply = () => invoke<PendingApply>("pending_apply");
export const winhaoPresetState = () => invoke<PresetState>("winhao_preset_state");
/** 写入 Winhao 的配置并重启 Claude Desktop，返回写了几项。 */
export const applyWinhaoPreset = () => invoke<number>("apply_winhao_preset");
/** 在访达 / 资源管理器里选中 Claude Desktop 正在用的配置文件（出问题时让用户发过来）。 */
export const revealClaudeConfig = () => invoke<void>("reveal_claude_config");
/** 该服务商当前提供的模型（models.dev，按发布日期新→旧）；认不出或未同步时为空。 */
export const availableModels = (targetUrl: string) =>
  invoke<AvailableModel[]>("available_models", { targetUrl });
