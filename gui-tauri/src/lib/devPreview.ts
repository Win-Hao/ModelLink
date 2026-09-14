// 浏览器预览用的 IPC mock（官方 @tauri-apps/api/mocks）。
// 仅在 `vite dev` 且非 Tauri 运行时（无 __TAURI_INTERNALS__）加载 —— 见 main.tsx；
// 生产构建里 import.meta.env.DEV 为 false，整个模块被摇树剔除。

import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppliedState, Config, LogEntry } from "@/lib/ipc";
import { flattenModels } from "@/lib/presets";

const now = Math.floor(Date.now() / 1000);

let store: Config = {
  providers: [
    {
      target_url: "https://api.deepseek.com/anthropic",
      api_key: "sk-mock-1234567890",
      models: [
        { name: "deepseek-v4-pro", to_1m: "auto", context_limit: 1_000_000, pricing_synced: { input: 0.435, output: 0.87 } },
        { name: "deepseek-v4-flash", to_1m: "", context_limit: 262_144, pricing_synced: { input: 0.14, output: 0.28 } },
      ],
      thinking_effort: "max",
    },
    {
      target_url: "https://api.moonshot.cn/anthropic",
      api_key: "sk-mock-0987654321",
      models: [
        // ?unpricedcfg=1 → 这个模型没查到官方价（花费整列不显示）
        {
          name: "kimi-k3",
          to_1m: "auto",
          context_limit: 1_048_576,
          ...(new URLSearchParams(location.search).has("unpricedcfg") ? {} : { pricing_synced: { input: 3, output: 15 } }),
        },
      ],
      thinking_effort: "",
    },
  ],
  last_applied_hash: "",
  last_applied_at: String(now - 4 * 3600),
  last_applied_pool: "2.1",
};

// ?many=1 → 20 个模型 / 6 家服务商（看长表滚动）
const MANY: [string, string[]][] = [
  ["https://api.deepseek.com/anthropic", ["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-r2"]],
  ["https://api.moonshot.cn/anthropic", ["kimi-k3", "kimi-k2.7-code", "kimi-latest"]],
  ["https://open.bigmodel.cn/api/anthropic", ["glm-5.3", "glm-5.3-flash", "glm-5v-turbo", "glm-4.7"]],
  ["https://coding.dashscope.aliyuncs.com/apps/anthropic", ["qwen3.7-plus", "qwen3.6-flash", "qwen3.8-max", "qwen3.8-flash", "qwen-coder"]],
  ["https://api.minimaxi.com/anthropic", ["MiniMax-M3", "MiniMax-M2.7"]],
  ["https://api.xiaomimimo.com/anthropic", ["mimo-v2.5-pro", "mimo-v2.5", "mimo-vl"]],
];
if (new URLSearchParams(location.search).has("many")) {
  store.providers = MANY.map(([url, names], i) => ({
    target_url: url,
    api_key: `sk-mock-${i}`,
    models: names.map((name, j) => ({
      name,
      to_1m: j === 0 ? "auto" : "",
      context_limit: j === 0 ? 1_000_000 : [262_144, 131_072, 200_000][j % 3],
    })),
    thinking_effort: "",
  }));
}
// 初始为 clean 态：applied hash = 当前内容摘要
store.last_applied_hash = mockHash(store);

// Claude Desktop 那边「上次应用」时写进去的映射
function snapshot(c: Config): AppliedState {
  return {
    found: true,
    provider: "gateway",
    gateway_url: `http://127.0.0.1:${c.port ?? 5678}`,
    models: flattenModels(c).map((m) => ({ slot: m.slot, label: m.name, supports_1m: m.to1m })),
  };
}
let applied = snapshot(store);

// ?empty=1 → 空配置（预览首启引导页）；?dirty=1 → 初始即 dirty（最后一个模型是应用之后才加的）
const params = new URLSearchParams(location.search);
if (params.has("empty")) {
  store = { providers: [], last_applied_hash: "", last_applied_at: "" };
}
if (params.has("dirty")) {
  store.last_applied_hash = "stale";
  applied = { ...applied, models: applied.models.slice(0, -1) };
}

// 最新的在数组末尾（与后端一致）
const L = (e: Partial<LogEntry> & Pick<LogEntry, "time" | "model">, i: number): LogEntry => ({
  id: i + 1,
  slot: e.model,
  status: 200,
  thinking: "",
  note: "",
  error: false,
  duration_ms: 1800,
  usage: { input_tokens: 6200, output_tokens: 900, cache_read_tokens: 1900, cache_write_tokens: 0 },
  cost_usd: 0.0184,
  detail: "",
  ...e,
});
const logs: LogEntry[] = [
  { time: "14:26:19", model: "kimi-k3", slot: "claude-opus-4-8", thinking: "max", note: "已整流 · 上游不认推理档位", duration_ms: 3900 },
  { time: "14:27:50", model: "deepseek-v4-flash", slot: "claude-sonnet-5", duration_ms: 1600, cost_usd: 0.0031 },
  { time: "14:28:33", model: "deepseek-v4-pro", slot: "claude-opus-5", thinking: "high", duration_ms: 2700 },
  { time: "14:29:12", model: "claude-ml-4", slot: "claude-ml-4", status: 400, note: "未映射槽位", error: true, duration_ms: 3, usage: null, cost_usd: null },
  { time: "14:29:58", model: "kimi-k3", slot: "claude-opus-4-8", thinking: "max", duration_ms: 4800, cost_usd: 0.1312 },
  { time: "14:30:41", model: "deepseek-v4-flash", slot: "claude-sonnet-5", thinking: "off", note: "标题生成 · 已省思考", duration_ms: 1100, cost_usd: 0.0004 },
  { time: "14:31:05", model: "kimi-k3", slot: "claude-opus-4-8", status: 401, duration_ms: 420, usage: null, cost_usd: null, detail: "Invalid Authentication" },
  { time: "14:31:20", model: "deepseek-v4-pro", slot: "claude-opus-5", thinking: "high", note: "已整流 · 思考预算过小", duration_ms: 2200 },
  { time: "14:31:52", model: "deepseek-v4-pro", slot: "claude-opus-5", thinking: "high", duration_ms: 1900 },
  { time: "14:32:07", model: "kimi-k3", slot: "claude-opus-4-8", thinking: "max", duration_ms: null, usage: null, cost_usd: null },
].map(L);

function mockHash(c: Config): string {
  return JSON.stringify(c.providers);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

mockIPC(async (cmd, payload) => {
  const args = payload as Record<string, unknown>;
  switch (cmd) {
    case "get_config":
      return structuredClone(store);
    case "save_config": {
      const next = structuredClone(args.config as Config);
      next.last_applied_hash = store.last_applied_hash;
      next.last_applied_at = store.last_applied_at;
      store = next;
      // 与后端一致：返回合并后的那份，前端据它算 dirty
      return next;
    }
    case "config_hash":
      return mockHash(args.config as Config);
    case "get_logs":
      return logs;
    case "get_log_stats":
      return {
        since: params.has("latestart") ? now - 3600 : now - 20 * 3600,
        requests: 412,
        failures: 9,
        duration_total_ms: 412 * 2600,
        duration_max_ms: 48_200,
        input_tokens: 3_680_000,
        output_tokens: 440_000,
        with_usage: 398,
        cost_usd: 2.47,
        priced: params.has("unpriced") ? 390 : 398,
      };
    case "gui_version":
      return "2.0.0";
    case "apply_to_claude":
      await sleep(1200);
      store.last_applied_hash = mockHash(store);
      store.last_applied_at = String(Math.floor(Date.now() / 1000));
      applied = snapshot(store);
      return "Applied! Claude Desktop is restarting...";
    case "test_provider":
      await sleep(700);
      return { ok: true, message: "Connection successful! (HTTP 200)" };
    case "force_quit_and_relaunch":
      return null;
    case "proxy_status":
      return { running: !params.has("portdown"), port: store.port ?? 5678 };
    case "available_models": {
      const url = String(args.targetUrl ?? "");
      if (url.includes("deepseek"))
        return [
          { id: "deepseek-v4-flash-vision-exp", context: 131_072 },
          { id: "deepseek-v4-pro", context: 1_000_000 },
          { id: "deepseek-v4-flash", context: 262_144 },
        ];
      if (url.includes("moonshot"))
        return [
          { id: "kimi-k3", context: 1_048_576 },
          { id: "kimi-k2.7-code", context: 262_144 },
          { id: "kimi-k2.7-code-highspeed", context: 262_144 },
          { id: "kimi-k2.6", context: 262_144 },
        ];
      return [];
    }
    case "applied_state":
      return applied;
    case "reveal_claude_config":
      return null;
    case "desktop_info":
      return { version: "1.46388.3", unavailable: [] };
    case "sync_pricing": {
      await sleep(600);
      return { ok: true, changed: 2, skipped: false, message: "", synced_at: String(Math.floor(Date.now() / 1000)) };
    }
    case "set_port": {
      await sleep(400);
      store.port = args.port as number;
      return { running: true, port: store.port };
    }
    case "plugin:app|version":
      return "2.0.0";
    case "plugin:autostart|is_enabled":
      return true;
    case "plugin:autostart|enable":
    case "plugin:autostart|disable":
      return null;
    case "plugin:updater|check":
      return null; // 无更新
    case "plugin:opener|open_url":
      return null;
    default:
      console.warn("[devPreview] unhandled IPC:", cmd, args);
      return null;
  }
});

console.info("[devPreview] Tauri IPC mocked for browser preview");
