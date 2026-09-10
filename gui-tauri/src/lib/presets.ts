import type { Config } from "@/lib/ipc";

// ============================================================
// 服务商预设与槽位常量 —— 数据自 v1 ui.html:272-360 平移，勿改。
// ============================================================

/** 镜像后端 config.rs::MAX_MODELS（2.1-B §3.6：8 → 20）。 */
export const MAX_MODELS = 20;

/** 镜像后端 config.rs::FAMILY_TIERS（app.asar 里的 Ba 数组，§3.5）。 */
export const FAMILY_TIERS = ["sonnet", "opus", "haiku", "fable", "mythos"] as const;

/** 「1M 上下文」的判定门槛：各家数字不一（Kimi 1048576 / 智谱 1000000），取下限。 */
export const ONE_M_CONTEXT = 1_000_000;

/** 已知这个模型装不下 1M，却开着 1M 开关。未知（没同步到）时不下结论。 */
export function claims1mItDoesNotHave(m: { to_1m: string; context_limit?: number }): boolean {
  return !!m.to_1m && m.context_limit !== undefined && m.context_limit < ONE_M_CONTEXT;
}

/** 上下文大小的人类可读写法：262144 → 256K。 */
export function formatContext(n?: number): string {
  if (!n) return "";
  return n >= ONE_M_CONTEXT ? `${Math.round(n / 1024 / 1024)}M` : `${Math.round(n / 1024)}K`;
}

/** §3.8：键 → 用户看得懂的能力名，用于「因版本过低不可用」提示。 */
export const KEY_FEATURE_NAMES: Record<string, string> = {
  chatTabEnabled: "Chat 页",
  labelOverride: "模型显示真名",
  inferenceModelPricingEnabled: "费用估算",
  inferenceModelPricing: "自定义费率",
  organizationInstructions: "组织级指令",
  inferenceStreamIdleTimeoutSec: "长生成防断流",
  egressProxyUrl: "网络代理",
  egressProxyPacUrl: "PAC 代理",
};

/** organizationInstructions 的硬上限（app.asar：D().trim().min(1).max(3e3)）。 */
export const ORG_INSTRUCTIONS_MAX = 3000;

/** 镜像后端 config.rs::DEFAULT_HEARTBEAT_SECS（§3.2）。 */
export const DEFAULT_HEARTBEAT_SECS = 15;

/** 镜像后端 config.rs::DEFAULT_USD_RATE（§五①）。 */
export const DEFAULT_USD_RATE = 7.2;

/** 镜像后端 config.rs::SLOT_POOL_VERSION —— 换槽位池时两边必须同时改。 */
export const SLOT_POOL_VERSION = "2.1";

/**
 * 槽位池（§2.1），镜像后端 config.rs::SLOT_POOL，顺序即分配优先级。
 *
 * `efforts` 是 **Claude Desktop 自己**那张按模型 ID 精确匹配的硬编码表（Vwt）里的档位，
 * 只用来在界面上告诉用户「这个模型在 Claude 里能不能调推理强度」。
 * ⚠️ 它决定的只是桌面端的选择器 UI —— 发给上游的参数一律不得从槽位名推断（§3.11.5）。
 */
export type Slot = { id: string; efforts: string[] };

export const SLOT_POOL: Slot[] = [
  // 一线：5 档 effort + auto 模式
  { id: "claude-opus-5", efforts: ["low", "medium", "high", "xhigh", "max"] },
  { id: "claude-sonnet-5", efforts: ["low", "medium", "high", "xhigh", "max"] },
  { id: "claude-opus-4-8", efforts: ["low", "medium", "high", "xhigh", "max"] },
  { id: "claude-opus-4-7", efforts: ["low", "medium", "high", "xhigh", "max"] },
  // 二线：4 档
  { id: "claude-opus-4-6", efforts: ["low", "medium", "high", "max"] },
  { id: "claude-sonnet-4-6", efforts: ["low", "medium", "high", "max"] },
  // 三线：仅 extended 模式，无 effort 选择器
  { id: "claude-sonnet-4-5", efforts: [] },
  { id: "claude-haiku-4-5", efforts: [] },
];

/** 第 n 个模型（0-based）占的槽位；池子用完走 claude-ml-{n} 溢出层（无选择器）。 */
export function slotId(index: number): string {
  return SLOT_POOL[index]?.id ?? `claude-ml-${index - SLOT_POOL.length + 1}`;
}

/** 该槽位在 Claude Desktop 里有几档推理强度可选（0 = 不显示选择器）。 */
export function slotEfforts(index: number): string[] {
  return SLOT_POOL[index]?.efforts ?? [];
}

export type Preset = {
  id: string;
  name: string;
  url: string;
  models: string[];
  thinkingOptions: string[];
};

export const PRESETS: Preset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    url: "https://api.deepseek.com/anthropic",
    models: ["deepseek-v4-pro", "deepseek-v4-flash"],
    thinkingOptions: ["", "off", "high", "max"],
  },
  {
    id: "kimi-code",
    name: "Kimi Code（订阅制）",
    url: "https://api.kimi.com/coding/",
    models: ["Kimi-k2.6"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "kimi",
    name: "Kimi 开放平台（按量付费）",
    url: "https://api.moonshot.cn/anthropic",
    models: ["kimi-k2.5", "kimi-k2.6"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "minimax",
    name: "MiniMax",
    url: "https://api.minimaxi.com/anthropic",
    models: ["MiniMax-M2.7", "MiniMax-M2.7-highspeed"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "qwen-coding",
    name: "百炼 Coding Plan",
    url: "https://coding.dashscope.aliyuncs.com/apps/anthropic",
    models: ["qwen3.6-plus", "qwen3-coder-next"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "qwen-token",
    name: "百炼 Token Plan",
    url: "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
    models: ["qwen3.6-plus", "qwen3-coder-next", "glm-5", "MiniMax-M2.5"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "glm",
    name: "GLM（智谱）",
    url: "https://open.bigmodel.cn/api/anthropic",
    models: ["glm-5.1", "glm-5-turbo", "glm-4.7", "glm-4.5-air"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "mimo",
    name: "mimo",
    url: "https://api.xiaomimimo.com/anthropic",
    models: ["mimo-v2.5-pro", "mimo-v2.5", "mimo-v2-pro", "mimo-v2-omni", "mimo-v2-flash"],
    thinkingOptions: ["", "off"],
  },
];

/** 预设网格 tile 上的域名短标（design-proposal §03）。 */
export function presetHost(p: Preset): string {
  try {
    const h = new URL(p.url).hostname;
    if (p.id === "qwen-coding") return "dashscope";
    if (p.id === "qwen-token") return "token-plan";
    return h;
  } catch {
    return p.url;
  }
}

export function matchPreset(url: string): Preset | null {
  if (!url) return null;
  const u = url.toLowerCase();
  for (const p of PRESETS) {
    try {
      const host = new URL(p.url).hostname;
      if (u.includes(host)) return p;
    } catch {
      /* ignore */
    }
  }
  return null;
}

export function getThinkingOptions(url: string): string[] {
  return matchPreset(url)?.thinkingOptions ?? ["", "off", "high", "max"];
}

export function getPresetModels(url: string): string[] {
  return matchPreset(url)?.models ?? [];
}

export const THINKING_LABELS: Record<string, string> = {
  "": "默认（不干预）",
  off: "关闭思考",
  high: "标准 (high)",
  max: "深度 (max)",
};

/**
 * 请求日志 / 链路 chip 上的思考标签（design.md §6.3）。
 * low/medium/xhigh 来自 Claude Desktop 原生 5 档选择器的透传值（2.1-A §3.10），
 * 桌面端 UI 上 xhigh 显示为 Extra。
 */
export const THINKING_TAGS: Record<string, string> = {
  "": "默认",
  off: "思考关",
  low: "轻度",
  medium: "中等",
  high: "标准",
  xhigh: "超高",
  max: "深度",
};

/** URL → 服务商显示名（平移 v1 detectProvider）。 */
export function detectProvider(url: string): string {
  if (!url) return "";
  const u = url.toLowerCase();
  if (u.includes("deepseek.com")) return "DeepSeek";
  if (u.includes("kimi.com")) return "Kimi Code";
  if (u.includes("moonshot")) return "Kimi";
  if (u.includes("xiaomimimo") || u.includes("mimo")) return "mimo";
  if (u.includes("minimaxi.com")) return "MiniMax";
  if (u.includes("openai.com")) return "OpenAI";
  if (u.includes("openrouter")) return "OpenRouter";
  if (u.includes("groq.com")) return "Groq";
  if (u.includes("together")) return "Together";
  if (u.includes("siliconflow")) return "SiliconFlow";
  if (u.includes("baichuan") || u.includes("百川")) return "Baichuan";
  if (u.includes("token-plan") && u.includes("aliyun")) return "百炼 Token Plan";
  if (u.includes("dashscope") || u.includes("aliyun")) return "百炼 Coding Plan";
  if (u.includes("zhipu") || u.includes("bigmodel")) return "GLM（智谱）";
  return "";
}

export function providerDisplayName(url: string, index: number): string {
  return detectProvider(url) || `服务商 ${index + 1}`;
}

/** 链路板 / 编辑器共用的槽位展开（镜像后端 flatten_config 语义：跳过空名、封顶 MAX_MODELS）。 */
export type FlatModel = {
  slot: string;
  /** 该槽位在 Claude Desktop 里的推理强度档位（空 = 无选择器）。 */
  efforts: string[];
  name: string;
  to1m: boolean;
  providerIndex: number;
  modelIndex: number;
};

export function flattenModels(config: Config): FlatModel[] {
  const out: FlatModel[] = [];
  let count = 0;
  config.providers.forEach((p, pi) => {
    p.models.forEach((m, mi) => {
      if (count < MAX_MODELS && m.name) {
        out.push({
          slot: slotId(count),
          efforts: slotEfforts(count),
          name: m.name,
          to1m: !!m.to_1m,
          providerIndex: pi,
          modelIndex: mi,
        });
        count += 1;
      }
    });
  });
  return out;
}

/** 模型行的槽位提示用「原始序号」（含未命名行，平移 v1 globalModelStart 行为）。 */
export function rawSlotForModel(config: Config, pi: number, mi: number): string {
  let idx = 0;
  for (let i = 0; i < pi; i++) idx += config.providers[i]?.models.length ?? 0;
  idx += mi;
  return idx < MAX_MODELS ? slotId(idx) : "";
}

/** 所有服务商模型总数（含未命名行，上限判定用，平移 v1 totalModels）。 */
export function totalModelsRaw(config: Config): number {
  return config.providers.reduce((s, p) => s + p.models.length, 0);
}

/** 「上次应用」时间显示：今天 → 今天 HH:MM，否则 M月D日 HH:MM。 */
export function formatAppliedAt(epochSecs?: string): string | null {
  if (!epochSecs) return null;
  const n = Number(epochSecs);
  if (!Number.isFinite(n) || n <= 0) return null;
  const d = new Date(n * 1000);
  const now = new Date();
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) return `今天 ${hm}`;
  return `${d.getMonth() + 1}月${d.getDate()}日 ${hm}`;
}
