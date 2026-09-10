// 浏览器预览用的 IPC mock（官方 @tauri-apps/api/mocks）。
// 仅在 `vite dev` 且非 Tauri 运行时（无 __TAURI_INTERNALS__）加载 —— 见 main.tsx；
// 生产构建里 import.meta.env.DEV 为 false，整个模块被摇树剔除。

import { mockIPC } from "@tauri-apps/api/mocks";
import type { Config, LogEntry } from "@/lib/ipc";

const now = Math.floor(Date.now() / 1000);

let store: Config = {
  providers: [
    {
      target_url: "https://api.deepseek.com/anthropic",
      api_key: "sk-mock-1234567890",
      models: [
        { name: "deepseek-v4-pro", to_1m: "auto" },
        { name: "deepseek-v4-flash", to_1m: "" },
      ],
      thinking_effort: "max",
    },
    {
      target_url: "https://api.moonshot.cn/anthropic",
      api_key: "sk-mock-0987654321",
      models: [{ name: "kimi-k2.5", to_1m: "auto" }],
      thinking_effort: "",
    },
  ],
  last_applied_hash: "",
  last_applied_at: String(now - 4 * 3600),
};
// 初始为 clean 态：applied hash = 当前内容摘要
store.last_applied_hash = mockHash(store);

// ?empty=1 → 空配置（预览首启引导页）；?dirty=1 → 初始即 dirty
const params = new URLSearchParams(location.search);
if (params.has("empty")) {
  store = { providers: [], last_applied_hash: "", last_applied_at: "" };
}
if (params.has("dirty")) {
  store.last_applied_hash = "stale";
}

const logs: LogEntry[] = [
  { time: "14:21:09", model: "kimi-k2.5", status: 200, thinking: "", note: "", error: false },
  {
    time: "14:25:31",
    model: "deepseek-v4-flash",
    status: 200,
    thinking: "",
    note: "",
    error: false,
  },
  {
    time: "14:27:55",
    model: "deepseek-v4-flash",
    status: 502,
    thinking: "off",
    note: "",
    error: false,
  },
  { time: "14:29:03", model: "kimi-k2.5[1m]", status: 200, thinking: "", note: "", error: false },
  // 2.1-A：桌面端 5 档透传 + effort 整流 + 未映射槽位三种新形态
  {
    time: "14:30:12",
    model: "deepseek-v4-pro",
    status: 200,
    thinking: "xhigh",
    note: "",
    error: false,
  },
  {
    time: "14:31:20",
    model: "MiniMax-M2.7",
    status: 200,
    thinking: "",
    note: "已自动修复：effort 不支持",
    error: false,
  },
  {
    time: "14:32:05",
    model: "claude-3-5-haiku-latest",
    status: 400,
    thinking: "",
    note: "未映射槽位",
    error: true,
  },
  {
    time: "14:32:47",
    model: "deepseek-v4-pro",
    status: 200,
    thinking: "max",
    note: "",
    error: false,
  },
];

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
      return null;
    }
    case "config_hash":
      return mockHash(args.config as Config);
    case "get_logs":
      return logs;
    case "gui_version":
      return "2.0.0";
    case "apply_to_claude":
      await sleep(1200);
      store.last_applied_hash = mockHash(store);
      store.last_applied_at = String(Math.floor(Date.now() / 1000));
      return "Applied! Claude Desktop is restarting...";
    case "test_provider":
      await sleep(700);
      return { ok: true, message: "Connection successful! (HTTP 200)" };
    case "force_quit_and_relaunch":
      return null;
    case "proxy_status":
      return { running: !params.has("portdown"), port: store.port ?? 5678 };
    case "probe_provider": {
      await sleep(1200);
      return {
        report: {
          ok: true,
          message: "",
          models_endpoint: true,
          upstream_efforts: ["low", "high", "max"],
          validates_model_name: false,
          accepts_claude_slot: true,
          effort_accepted: [
            ["low", true],
            ["medium", true],
            ["high", true],
            ["xhigh", true],
            ["max", true],
          ],
          thinking_variants: [
            ["adaptive", true],
            ["enabled", true],
            ["disabled", false],
          ],
          prompt_caching: true,
          accepts_1m_beta: true,
          elapsed_ms: 4200,
        },
        headlines: [
          "⚠ 这家会**静默回落**：请求一个不存在的模型也返回 200，直接给默认模型。",
          "✓ 五档推理强度全部接受，桌面端选择器可直接用（代理原样透传）。",
        ],
      };
    }
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
