import { invoke } from "@tauri-apps/api/core";

// ============================================================
// 后端 IPC 类型化封装 —— 对应 src-tauri 的 #[tauri::command]。
// 契约见 docs/gui-rebuild-tauri.md §5。
// 注意：Config 内部字段是 serde 原名（snake_case，兼容红线 #2）。
// ============================================================

export type ModelEntry = {
  name: string;
  /** 非空（v1 里为 "auto"）表示该模型开启 1M 上下文变体。 */
  to_1m: string;
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
  /** 兼容模式：未映射的槽位回落到第一个模型（v1 静默行为），默认关。 */
  compat_fallback?: boolean;
};

export type ProxyStatus = { running: boolean; port: number };

export type LogEntry = {
  time: string;
  model: string;
  status: number;
  /** 实际发给上游的推理强度（""=未发 / "off" / low…max）。 */
  thinking: string;
  /** 附注：整流标记 / 未映射槽位等，多条以 " · " 连接。 */
  note: string;
  /** ModelLink 自己判定为错误的请求（标红）。 */
  error: boolean;
};

export type TestResult = { ok: boolean; message: string };

export const guiVersion = () => invoke<string>("gui_version");
export const getConfig = () => invoke<Config>("get_config");
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const configHash = (config: Config) => invoke<string>("config_hash", { config });
export const testProvider = (targetUrl: string, apiKey: string, model: string) =>
  invoke<TestResult>("test_provider", { targetUrl, apiKey, model });
export const applyToClaude = () => invoke<string>("apply_to_claude");
export const getLogs = () => invoke<LogEntry[]>("get_logs");
export const proxyStatus = () => invoke<ProxyStatus>("proxy_status");
export const setPort = (port: number) => invoke<ProxyStatus>("set_port", { port });
