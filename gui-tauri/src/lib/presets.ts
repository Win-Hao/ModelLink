import type { Config } from "@/lib/ipc";

// ============================================================
// 服务商预设与槽位常量 —— 数据自 v1 ui.html:272-360 平移，勿改。
// ============================================================

export const MAX_MODELS = 8;

export const ANTHROPIC_SLOTS = [
  "claude-3-opus-latest",
  "claude-3-5-sonnet-latest",
  "claude-3-sonnet-20240229",
  "claude-3-haiku-20240307",
  "claude-3-5-haiku-latest",
  "claude-3-opus-20240229",
  "claude-3-5-sonnet-20241022",
  "claude-3-5-sonnet-20240620",
] as const;

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

/** 链路板 / 编辑器共用的槽位展开（镜像后端 flatten_config 语义：跳过空名、封顶 8）。 */
export type FlatModel = {
  slot: string;
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
          slot: ANTHROPIC_SLOTS[count],
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
  return idx < ANTHROPIC_SLOTS.length ? ANTHROPIC_SLOTS[idx] : "";
}

/** 所有服务商模型总数（含未命名行，8 上限判定用，平移 v1 totalModels）。 */
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
