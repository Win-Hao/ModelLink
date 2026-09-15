import { MODELS_SNAPSHOT } from "@/lib/modelsSnapshot";
import type { AppliedModel, AvailableModel, Config, ModelEntry } from "@/lib/ipc";

// ============================================================
// 服务商预设与槽位常量 —— 数据自 v1 ui.html:272-360 平移，勿改。
// ============================================================

/**
 * 镜像后端 config.rs::MAX_MODELS = SLOT_POOL 里的名字数。2.1 曾放到 20（第 9 个起用 claude-ml-N 占位名），
 * 2.2 收回到 8：Claude 认得的真名只有 8 个，占位名不能调思考，实际也用不到这么多。
 */
export const MAX_MODELS = 8;

/** 「1M 上下文」的判定门槛：各家数字不一（Kimi 1048576 / 智谱 1000000），取下限。 */
export const ONE_M_CONTEXT = 1_000_000;

/** 已知这个模型装不下 1M，却开着 1M 开关。未知（没同步到）时不下结论。 */
export function claims1mItDoesNotHave(m: { to_1m: string; context_limit?: number }): boolean {
  return !!m.to_1m && m.context_limit !== undefined && m.context_limit < ONE_M_CONTEXT;
}

/**
 * 上下文大小的人类可读写法。各家给的数有十进制也有二进制：
 * 200000 → 200K、262144 → 256K、1000000 / 1048576 → 1M。
 */
export function formatContext(n?: number): string {
  if (!n) return "";
  const base = n % 1000 === 0 ? 1000 : 1024;
  if (n >= ONE_M_CONTEXT) return `${+(n / base / base).toFixed(1)}M`;
  return `${Math.round(n / base)}K`;
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

/** 镜像后端 config.rs::SLOT_POOL_VERSION —— 换槽位池时两边必须同时改。 */
export const SLOT_POOL_VERSION = "2.1";

/**
 * 能写进 Claude 的名字（§2.1），镜像后端 config.rs::SLOT_POOL，顺序即新模型默认拿名字的先后。
 *
 * 只用来在界面上告诉用户「这个模型在 Claude 里能用什么」：
 * - `efforts`：**Claude Desktop 自己**那张按模型 ID 精确匹配的硬编码表（Vwt）里的思考档位
 * - `auto`：走网关时有没有 Auto 模式（桌面端自带的 Claude Code 引擎按名字判断，见 config.rs::SLOT_POOL）
 *
 * ⚠️ 它决定的只是桌面端的选择器 UI —— 发给上游的参数一律不得从槽位名推断（§3.11.5）。
 */
export type Slot = { id: string; efforts: string[]; auto: boolean };

export const SLOT_POOL: Slot[] = [
  // 一线：Auto 模式 + 5 档思考
  { id: "claude-opus-5", efforts: ["low", "medium", "high", "xhigh", "max"], auto: true },
  { id: "claude-sonnet-5", efforts: ["low", "medium", "high", "xhigh", "max"], auto: true },
  { id: "claude-opus-4-8", efforts: ["low", "medium", "high", "xhigh", "max"], auto: true },
  { id: "claude-opus-4-7", efforts: ["low", "medium", "high", "xhigh", "max"], auto: true },
  // 二线：4 档思考，没有 Auto 模式
  { id: "claude-opus-4-6", efforts: ["low", "medium", "high", "max"], auto: false },
  { id: "claude-sonnet-4-6", efforts: ["low", "medium", "high", "max"], auto: false },
  // 三线：不能调思考，没有 Auto 模式
  { id: "claude-sonnet-4-5", efforts: [], auto: false },
  { id: "claude-haiku-4-5", efforts: [], auto: false },
];

/** 能写进 Claude 的全部名字（镜像 config.rs::SLOT_POOL），顺序即新模型默认拿名字的先后。 */
export const SLOT_NAMES: readonly string[] = SLOT_POOL.map((s) => s.id);

/** 这个名字在 Claude 里能用什么。 */
export function slotInfo(slot: string): { efforts: string[]; auto: boolean } {
  const s = SLOT_POOL.find((x) => x.id === slot);
  return { efforts: s?.efforts ?? [], auto: s?.auto ?? false };
}

/** 一句话说这个名字在 Claude 里能用什么（服务商页「在 Claude 里」那一列、换名字的下拉）。 */
export function slotAbility(slot: string): string {
  const { efforts, auto } = slotInfo(slot);
  return `${auto ? "Auto 模式" : "没有 Auto 模式"} · ${efforts.length > 0 ? `思考 ${efforts.length} 档` : "思考不能调"}`;
}

/** 会写进 Claude 的模型：有名字的前 MAX_MODELS 个。 */
function namedModels(config: Config): ModelEntry[] {
  return config.providers.flatMap((p) => p.models).filter((m) => m.name).slice(0, MAX_MODELS);
}

function needsSlots(config: Config): boolean {
  const seen = new Set<string>();
  return namedModels(config).some((m) => {
    if (!m.slot || !SLOT_NAMES.includes(m.slot) || seen.has(m.slot)) return true;
    seen.add(m.slot);
    return false;
  });
}

/**
 * 给会写进 Claude 的模型定下名字，就地改（镜像 config.rs::normalize_slots）。
 * 认得出、没重复的原样保留；其余有名字的按能力从高到低拿第一个空位；没名字的行、超出上限的不占名字。
 * 增删模型都不挪别人的 —— Claude 里选着某个名字的对话，不会悄悄换成别的模型。
 */
export function normalizeSlots(config: Config): void {
  const named = new Set(namedModels(config));
  const used = new Set<string>();
  const waiting: ModelEntry[] = [];
  for (const m of config.providers.flatMap((p) => p.models)) {
    if (!named.has(m)) {
      delete m.slot;
    } else if (m.slot && SLOT_NAMES.includes(m.slot) && !used.has(m.slot)) {
      used.add(m.slot);
    } else {
      waiting.push(m);
    }
  }
  const free = SLOT_NAMES.filter((n) => !used.has(n));
  waiting.forEach((m, i) => {
    m.slot = free[i];
  });
}

export type Preset = {
  id: string;
  name: string;
  url: string;
  /** 同一家的其它 API 域名（服务商换域名时，照新文档填的用户也要认得出来） */
  hosts?: string[];
  models: string[];
  thinkingOptions: string[];
};

export const PRESETS: Preset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    url: "https://api.deepseek.com/anthropic",
    // 两条都还是当前主力（v4-pro 2026-08-12 / v4-flash 2026-07-31）。
    // 不收 deepseek-v4-flash-vision-exp —— 实验性视觉模型，不适合当默认。
    models: ["deepseek-v4-pro", "deepseek-v4-flash"],
    thinkingOptions: ["", "off", "high", "max"],
  },
  {
    id: "kimi-code",
    name: "Kimi Code（订阅制）",
    url: "https://api.kimi.com/coding/",
    // 订阅制现在提供的是 k3（2026-07-16，1M 上下文）与 k3-256k。
    // 原先写的 Kimi-k2.6 是开放平台的型号，在这家的目录里根本不存在。
    models: ["k3", "k3-256k"],
    // 官方文档：output_config.effort 支持 low / high / max，默认 max。
    // medium 与 xhigh 是 Claude Desktop 的档位，Kimi 在网关侧映射掉，这里不列。
    thinkingOptions: ["", "off", "low", "high", "max"],
  },
  {
    id: "kimi",
    name: "Kimi 开放平台（按量付费）",
    url: "https://api.moonshot.cn/anthropic",
    // kimi-k3 是当前旗舰（1M 上下文），kimi-k2.7-code 便宜三倍且专做编码。
    models: ["kimi-k3", "kimi-k2.7-code"],
    // 同上，官方文档所列的三档
    thinkingOptions: ["", "off", "low", "high", "max"],
  },
  {
    id: "minimax",
    name: "MiniMax",
    url: "https://api.minimaxi.com/anthropic",
    // 官方文档里的 API 地址已换成 api.minimax.cn（旧域名仍可用，两者是同一个后端）
    hosts: ["api.minimax.cn"],
    // M3（2026-06-01）是当前旗舰，且是这家唯一有 1M 上下文的。
    models: ["MiniMax-M3", "MiniMax-M2.7"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "qwen-coding",
    name: "百炼 Coding Plan",
    url: "https://coding.dashscope.aliyuncs.com/apps/anthropic",
    // qwen3.7-plus 是套餐内当前主力；qwen3.6-flash 更快更省。
    models: ["qwen3.7-plus", "qwen3.6-flash"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "qwen-token",
    name: "百炼 Token Plan",
    url: "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
    // qwen3.8 系列是当前主力（max 强、flash 快）。
    // 不收 qwen3.8-max-preview（预览版）与 happyhorse-*（视频生成，非对话模型）。
    models: ["qwen3.8-max", "qwen3.8-flash", "glm-5.2"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "glm",
    name: "GLM（智谱）",
    url: "https://open.bigmodel.cn/api/anthropic",
    // glm-5.3（2026-08-14）是当前旗舰且有 1M 上下文；flash 便宜近 20 倍。
    // 不收 glm-5v-turbo（视觉，且贵 3.5 倍）。
    models: ["glm-5.3", "glm-5.3-flash"],
    thinkingOptions: ["", "off"],
  },
  {
    id: "mimo",
    name: "mimo",
    url: "https://api.xiaomimimo.com/anthropic",
    // v2.5 系列（2026-04-22，1M 上下文）。
    // 不收 mimo-*-tts（语音合成，上下文只有 8K，不是对话模型）。
    models: ["mimo-v2.5-pro", "mimo-v2.5"],
    thinkingOptions: ["", "off"],
  },
];

/** 预设网格格子上显示的域名（真实地址，两个百炼方案靠前缀区分）。 */
export function presetHost(p: Preset): string {
  try {
    return new URL(p.url).hostname;
  } catch {
    return p.url;
  }
}

/**
 * 各家「创建 / 查看 API 密钥」的官方页面，服务商页密钥框旁边的链接用。
 *
 * 只收官方文档里写明的地址（2026-09-14 逐家核对；控制台都要登录，证据是文档里的链接原文）。
 * 同一个 API 地址按量付费和订阅套餐共用、走哪条线由密钥决定的（MiniMax、智谱），
 * 两个入口都给并写明是哪条线 —— 只给一个会把另一条线的用户带去错的地方，而两边的密钥不通用。
 */
export type KeyPage = { url: string; plan?: string };

const KEY_PAGES: Record<string, KeyPage[]> = {
  deepseek: [{ url: "https://platform.deepseek.com/api_keys" }],
  "kimi-code": [{ url: "https://www.kimi.com/code/console" }],
  kimi: [{ url: "https://platform.kimi.com/console/api-keys" }],
  minimax: [
    { plan: "按量付费", url: "https://platform.minimaxi.com/user-center/basic-information/interface-key" },
    { plan: "Token Plan", url: "https://platform.minimaxi.com/user-center/payment/token-plan" },
  ],
  "qwen-coding": [{ url: "https://bailian.console.aliyun.com/cn-beijing/subscription/coding-plan" }],
  // 团队版成员的密钥由管理员生成，没有自助页面；个人版是这个
  "qwen-token": [{ url: "https://bailian.console.aliyun.com/cn-beijing/subscription/token-plan/personal" }],
  glm: [
    { plan: "按量付费", url: "https://bigmodel.cn/usercenter/proj-mgmt/apikeys" },
    { plan: "Coding Plan", url: "https://bigmodel.cn/coding-plan/personal/overview" },
  ],
  mimo: [{ url: "https://platform.xiaomimimo.com/#/console/api-keys" }],
};

export function keyPagesFor(url: string): KeyPage[] {
  const p = matchPreset(url);
  return (p && KEY_PAGES[p.id]) || [];
}

export function matchPreset(url: string): Preset | null {
  if (!url) return null;
  const u = url.toLowerCase();
  for (const p of PRESETS) {
    try {
      const hosts = [new URL(p.url).hostname, ...(p.hosts ?? [])];
      if (hosts.some((host) => u.includes(host))) return p;
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

/**
 * URL → models.dev 服务商 ID，镜像后端 `models_dev::provider_id_for_url`。
 * 只给快照兜底用 —— 运行时数据由后端按同一张表查好后返回。
 */
function modelsDevProviderId(url: string): string | undefined {
  const u = url.toLowerCase();
  const table: [string, string][] = [
    ["api.kimi.com", "kimi-for-coding"],
    ["moonshot", "moonshotai-cn"],
    ["deepseek.com", "deepseek"],
    ["minimaxi.com", "minimax-cn"],
    ["minimax.cn", "minimax-cn"],
    ["minimax.io", "minimax"],
    // 小米两条线：Token Plan 有独立域名，api.xiaomimimo.com 是按量付费；都要排在通用的 "token-plan" 前面
    ["token-plan-cn.xiaomimimo", "xiaomi-token-plan-cn"],
    ["xiaomimimo", "xiaomi"],
    ["coding.dashscope", "alibaba-coding-plan-cn"],
    ["token-plan", "alibaba-token-plan-cn"],
    ["dashscope", "alibaba-cn"],
    ["bigmodel.cn", "zhipuai"],
    ["zhipu", "zhipuai"],
  ];
  return table.find(([host]) => u.includes(host))?.[1];
}

/**
 * 模型选择器的候选清单，三级兜底：
 * 1. 运行时从 models.dev 同步来的（最新，带上下文上限；首次打开 / 断网时没有）
 * 2. 发版时打包进来的快照（`npm run sync-models` 生成，只有模型名）
 * 3. 手写在预设里的那份（最后的兜底；会过期，实测 Kimi Code 那条落后过两代）
 */
export function modelOptions(
  url: string,
  live: AvailableModel[] | undefined,
): { options: AvailableModel[]; source: "live" | "snapshot" | "preset" | "none" } {
  if (live?.length) return { options: live, source: "live" };
  const pid = modelsDevProviderId(url);
  const snap = pid ? MODELS_SNAPSHOT[pid] : undefined;
  if (snap?.length) return { options: snap.map((id) => ({ id, context: null })), source: "snapshot" };
  const preset = getPresetModels(url);
  if (preset.length) return { options: preset.map((id) => ({ id, context: null })), source: "preset" };
  return { options: [], source: "none" };
}

/** 默认思考深度下拉的选项名（给用户看，不带 low / max 这类参数名）。 */
export const THINKING_LABELS: Record<string, string> = {
  "": "按服务商默认",
  off: "不思考",
  low: "轻度",
  medium: "中等",
  high: "标准",
  xhigh: "较深",
  max: "深度",
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
  if (u.includes("minimaxi.com") || u.includes("minimax.cn")) return "MiniMax";
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
  /** 它在 Claude 里用的名字（存在配置里，见 normalizeSlots） */
  slot: string;
  /** 这个名字在 Claude Desktop 里的思考档位（空 = 无选择器）。 */
  efforts: string[];
  name: string;
  to1m: boolean;
  providerIndex: number;
  modelIndex: number;
};

export function flattenModels(config: Config): FlatModel[] {
  // 草稿每次改动都规范过；没规范过的（刚读进来的老数据）现补一份，规则相同
  if (needsSlots(config)) {
    const normalized = structuredClone(config);
    normalizeSlots(normalized);
    config = normalized;
  }
  const out: FlatModel[] = [];
  let count = 0;
  config.providers.forEach((p, pi) => {
    p.models.forEach((m, mi) => {
      if (count < MAX_MODELS && m.name) {
        out.push({
          slot: m.slot!,
          efforts: slotInfo(m.slot!).efforts,
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

/**
 * 当前槽位映射和 Claude Desktop 实际写着的逐条比对。
 *
 * 比的是 Claude 那边看得见的东西：槽位、显示名、有没有 1M 变体。
 * 密钥 / 地址这类改动代理立刻就用上了，不会让某个槽位「对不上」。
 * 老版本写入的条目没有显示名，这时只比槽位和 1M —— 缺的字段不当作不一致。
 */
export function diffApplied(
  flat: FlatModel[],
  applied: AppliedModel[],
): { pending: Set<string>; removed: number } {
  const bySlot = new Map(applied.map((a) => [a.slot, a]));
  const pending = new Set<string>();
  for (const row of flat) {
    const a = bySlot.get(row.slot);
    if (!a || a.supports_1m !== row.to1m || (a.label !== "" && a.label !== row.name)) {
      pending.add(row.slot);
    }
  }
  const current = new Set(flat.map((r) => r.slot));
  const removed = applied.filter((a) => !current.has(a.slot)).length;
  return { pending, removed };
}

/** 选好了模型的行数（不含没选模型的空行）。超过 MAX_MODELS 时，多出来的不会写进 Claude。 */
export function namedModelCount(config: Config): number {
  return config.providers.reduce((s, p) => s + p.models.filter((m) => m.name).length, 0);
}

/** 所有服务商模型总数（含未命名行，上限判定用，平移 v1 totalModels）。 */
export function totalModelsRaw(config: Config): number {
  return config.providers.reduce((s, p) => s + p.models.length, 0);
}

/** 相对时间：刚刚 / N 分钟前 / N 小时前 / N 天前，一周以上写日期。 */
export function formatSince(epochSecs?: string, nowMs = Date.now()): string | null {
  const n = Number(epochSecs);
  if (!epochSecs || !Number.isFinite(n) || n <= 0) return null;
  const mins = Math.floor((nowMs / 1000 - n) / 60);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins} 分钟前`;
  if (mins < 24 * 60) return `${Math.floor(mins / 60)} 小时前`;
  if (mins < 7 * 24 * 60) return `${Math.floor(mins / 60 / 24)} 天前`;
  const d = new Date(n * 1000);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

/**
 * 「应用」失败时后端给的是英文校验话术（与 v1 保持一致，回归套件按字节比对），
 * 界面上换成用户看得懂的说法；认不出的原样显示。
 */
export function applyErrorText(message: string, config: Config | null): string {
  const m = message.match(/^Provider (\d+) (.+)$/);
  if (m) {
    const i = Number(m[1]) - 1;
    const who = `「${providerDisplayName(config?.providers[i]?.target_url ?? "", i)}」`;
    const rest = m[2];
    if (rest === "has no API URL.") return `${who}还没填 API 地址`;
    if (rest.startsWith("URL must start with")) return `${who}的 API 地址要以 http:// 或 https:// 开头`;
    if (rest === "has no API key.") return `${who}还没填 API 密钥`;
    if (rest === "has no models.") return `${who}还没有模型`;
    if (rest === "has a model with empty name.") return `${who}有一个模型没填名字`;
  }
  if (message === "Please add at least one provider.") return "还没有添加服务商";
  return message;
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
