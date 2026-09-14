import type { Config, LogEntry, Usage } from "@/lib/ipc";
import { flattenModels } from "@/lib/presets";

// 请求日志页的格式化与判定（design-2.2.md §6.3）。

/** 一条日志的多个标记之间的分隔（与 proxy.rs 的 NOTE_SEPARATOR 一致；标记文字里自带「 · 」）。 */
const NOTE_SEPARATOR = "；";

/** 这些标记说明请求真出了错（红描边）；「已整流」开头的是自动修好了（绿描边）。 */
const BAD_NOTES = ["未映射槽位", "整流未生效", "整流后上游仍出错", "连不上服务商", "服务商没填"];

export type TagTone = "ok" | "bad" | "neutral";

/**
 * 标记在界面上的说法。后端写的是开发者口径（「整流」「未映射槽位」，回归套件与错误响应体都按它比对），
 * 这里只换显示文字，判定仍按原文。
 */
const NOTE_WORDS: [string, string][] = [
  ["整流后上游仍出错", "自动修复后服务商仍报错"],
  ["整流未生效", "自动修复没成功"],
  ["已整流", "已自动修复"],
  ["未映射槽位", "没有对应的模型"],
  ["上游不认推理档位", "服务商不认思考深度"],
  ["思考预算过小", "思考预算太小"],
];

function humanize(raw: string): string {
  return NOTE_WORDS.reduce((t, [from, to]) => t.replace(from, to), raw);
}

export function noteTags(note: string): { raw: string; text: string; tone: TagTone }[] {
  return note
    .split(NOTE_SEPARATOR)
    .filter(Boolean)
    .map((raw) => ({
      raw,
      text: humanize(raw),
      tone: raw.startsWith("已整流")
        ? "ok"
        : BAD_NOTES.some((b) => raw.startsWith(b))
          ? "bad"
          : "neutral",
    }));
}

export const isFailure = (e: LogEntry) => e.error || e.status >= 400;

export const isRectified = (e: LogEntry) => noteTags(e.note).some((t) => t.tone === "ok");

/** 输入侧合计：上游的 input_tokens 不含缓存命中与缓存写入，用户看的是一共发了多少。 */
export const promptTokens = (u: Usage) => u.input_tokens + u.cache_read_tokens + u.cache_write_tokens;

/** 840 → 840 · 12400 → 12.4k · 412000 → 412k · 1234567 → 1.23M */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 100_000) return `${+(n / 1000).toFixed(1)}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${+(n / 1_000_000).toFixed(2)}M`;
}

/** 3 → <0.1s · 380 → 0.4s · 65000 → 1m05s */
export function formatDuration(ms: number): string {
  if (ms < 50) return "<0.1s";
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

/** 美元：订阅制是 0；不到一分钱的保留四位，免得全显示成 $0.00。 */
export function formatUsd(v: number): string {
  if (v === 0) return "$0";
  if (v < 0.01) return `$${v.toFixed(4)}`;
  return `$${v.toFixed(2)}`;
}

/**
 * 当前接进 Claude 的每个模型都有输入 / 输出价。
 * 与写进 Claude 的费率表同一口径：只要有一个没价，花费就整列不显示 —— 半真半假的账单比没有更糟。
 */
export function pricingComplete(config: Config | null): boolean {
  if (!config) return false;
  const rows = flattenModels(config);
  return (
    rows.length > 0 &&
    rows.every((r) => {
      const p = config.providers[r.providerIndex].models[r.modelIndex].pricing_synced;
      return p?.input !== undefined && p?.output !== undefined;
    })
  );
}

/** 这个槽位（可能带 [1m]）现在归哪个服务商；找不到为 undefined。 */
export function providerIndexForSlot(config: Config | null, slot: string): number | undefined {
  if (!config) return undefined;
  const bare = slot.replace(/\[1m\]$/, "");
  return flattenModels(config).find((f) => f.slot === bare)?.providerIndex;
}
