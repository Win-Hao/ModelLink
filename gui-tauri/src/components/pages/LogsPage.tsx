import { Fragment, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";

import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { appliedState, getLogStats, getLogs, type LogEntry, type TodayStats } from "@/lib/ipc";
import {
  bareSlot,
  formatDuration,
  formatTokens,
  formatUsd,
  isFailure,
  isRectified,
  noteTags,
  pricingComplete,
  promptTokens,
  providerIndexForSlot,
} from "@/lib/logs";
import { THINKING_TAGS } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

// 数据行列宽（design-2.2.md §6.3）：时间 62 · 状态点 6 · 模型 flex · 耗时 70 · token 98 · 花费 72 · 状态码 40
const COL = {
  time: "w-[62px] flex-none",
  dot: "w-1.5 flex-none",
  model: "flex min-w-0 flex-1 items-center gap-2",
  ms: "w-[70px] flex-none text-right",
  tokens: "w-[98px] flex-none text-right",
  cost: "w-[72px] flex-none text-right",
  code: "w-10 flex-none text-right",
};

type Filter = "all" | "failed" | "rectified" | `model:${string}`;

/** 摘要横带里的一格：数字与标签同基线，右边一句补充。 */
function Stat({ value, unit, label, note, title, className }: {
  value: ReactNode;
  unit?: string;
  label: string;
  note?: ReactNode;
  title?: string;
  className?: string;
}) {
  return (
    <div
      className={cn("flex min-w-0 flex-1 items-baseline gap-[9px] px-5 [&+&]:shadow-[inset_1px_0_0_var(--hair)]", className)}
      title={title}
    >
      <span className="mono flex-none text-stat">
        {value}
        {unit && <small className="ml-px text-[12.5px] font-medium tracking-normal text-fg3">{unit}</small>}
      </span>
      <span className="flex-none text-[12px] text-fg2">{label}</span>
      {note && <span className="ml-auto truncate text-right text-[11.5px] text-fg3">{note}</span>}
    </div>
  );
}

function Summary({ stats, showCost }: { stats: TodayStats | undefined; showCost: boolean }) {
  const s = stats;
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000;
  // ModelLink 今天中途才启动：「今日」其实是从启动那一刻算起的，说清楚
  const lateStart = !!s && s.since > midnight + 60;
  const day = lateStart ? "启动后" : "今日";
  const sinceTitle = lateStart
    ? `ModelLink 今天 ${new Date(s!.since * 1000).toTimeString().slice(0, 5)} 才启动，之前的请求没有记录`
    : undefined;
  const has = !!s && s.requests > 0;
  const rate = has ? ((s.requests - s.failures) / s.requests) * 100 : 0;
  // 有失败时向下取整：别把 99.8% 显示成 100%
  const rateText = s && s.failures > 0 ? Math.floor(rate) : Math.round(rate);
  const costReady = showCost && !!s && s.priced === s.with_usage;

  return (
    <div className="panel mb-2.5 flex h-[62px] flex-none items-center">
      <Stat
        value={has ? rateText : "—"}
        unit={has ? "%" : undefined}
        label="成功率"
        note={has ? (s.failures > 0 ? `${s.failures} 条失败` : `${s.requests} 条全部成功`) : `${day}还没有请求`}
        title={sinceTitle}
      />
      <Stat
        value={has ? (s.duration_total_ms / s.requests / 1000).toFixed(1) : "—"}
        unit={has ? "s" : undefined}
        label="平均耗时"
        note={has ? `最慢 ${formatDuration(s.duration_max_ms)}` : undefined}
        title={sinceTitle}
      />
      <Stat
        value={has ? formatTokens(s.input_tokens + s.output_tokens) : "—"}
        label={`${day} token`}
        note={has ? `入 ${formatTokens(s.input_tokens)} · 出 ${formatTokens(s.output_tokens)}` : undefined}
        title={
          [
            sinceTitle,
            s && s.with_usage < s.requests
              ? `有 ${s.requests - s.with_usage} 条请求没拿到用量（中途停止、出错，或服务商没返回），没算进来`
              : undefined,
          ]
            .filter(Boolean)
            .join("\n") || undefined
        }
        className="flex-[1.35]"
      />
      {costReady ? (
        <Stat
          value={has ? formatUsd(s.cost_usd) : "—"}
          label={`${day}花费`}
          note="官方价"
          title={["按 models.dev 上各家服务商的官方价计算", sinceTitle].filter(Boolean).join("\n")}
        />
      ) : (
        // 费率不齐：整格不显示，换成一句说明 —— 绝不估算
        <div className="flex min-w-0 flex-1 items-baseline px-5 shadow-[inset_1px_0_0_var(--hair)]">
          <span className="text-[12px] leading-[1.5] text-fg3">
            {showCost ? "今天有请求的模型没有费率，不显示花费" : "有模型没查到官方价，不显示花费"}
          </span>
        </div>
      )}
    </div>
  );
}

/** 失败行下方的说明：说人话 + 一个实际出口。 */
function FailureNote({ entry }: { entry: LogEntry }) {
  const { draft, setPage, gotoProvider } = useAppStore();
  const appliedQ = useQuery({ queryKey: ["applied-state"], queryFn: appliedState });
  const pi = providerIndexForSlot(draft, entry.slot);
  const toProvider = () => (pi !== undefined ? gotoProvider(pi) : setPage("providers"));
  const detail = entry.detail && (
    <code className="mono block text-[12px] break-all text-fg2">{entry.detail}</code>
  );
  const tags = noteTags(entry.note).map((t) => t.raw);
  const bare = bareSlot(entry.slot);

  let title: string;
  let body: ReactNode;
  let action: { label: string; run: () => void } | undefined;

  if (tags.includes("未映射槽位")) {
    const stillInClaude = appliedQ.data?.models.some((m) => m.slot === bare);
    title = "Claude 请求的模型，在 ModelLink 里没有对应";
    if (stillInClaude) {
      body = "这个模型已经从 ModelLink 里删掉了，但 Claude Desktop 里还留着。应用一次，它就会从 Claude 里消失。";
      action = { label: "去概览页应用 →", run: () => setPage("overview") };
    } else {
      body = (
        <>
          多半是之前的对话还在用一个现在没有映射的代号。在 Claude 里给这个对话换一个模型，或者在「服务商」页把某个模型的映射模型改成它。ModelLink
          直接报错，不会悄悄换成别的模型回答。
          <code className="mono block text-[12px] text-fg3">Claude 请求的名字：{entry.slot.replace(/\[1m\]$/, "")}</code>
        </>
      );
      action = { label: "去「服务商」页改映射模型 →", run: () => setPage("providers") };
    }
  } else if (tags.some((t) => t.startsWith("连不上服务商"))) {
    title = "连不上服务商";
    body = <>网络不通，或者 API 地址填错了。{detail}</>;
    action = { label: "去「服务商」页检查地址 →", run: toProvider };
  } else if (tags.some((t) => t.startsWith("服务商没填"))) {
    title = "这个模型所在的服务商还没填 API 地址";
    body = "填上地址之后，这个模型的请求就能发出去了。";
    action = { label: "去填地址 →", run: toProvider };
  } else if (entry.status === 401 || entry.status === 403) {
    title = `服务商拒绝了这个密钥（HTTP ${entry.status}）`;
    body = <>密钥可能填错了、过期了，或者账户余额不足。{detail}</>;
    action = { label: "去「服务商」页检查密钥 →", run: toProvider };
  } else if (entry.status === 404) {
    title = "服务商说这个地址不存在（HTTP 404）";
    body = <>API 地址可能填错了。{detail}</>;
    action = { label: "去「服务商」页检查地址 →", run: toProvider };
  } else if (entry.status === 429) {
    title = "服务商限流了（HTTP 429）";
    body = <>请求太频繁，或者额度用完了。稍等一会儿再试；一直这样就去服务商后台看看额度。{detail}</>;
  } else if (entry.status >= 500) {
    title = `服务商那边出错了（HTTP ${entry.status}）`;
    body = <>不是你这边的配置问题，通常过一会儿就好。{detail}</>;
  } else {
    title = `服务商拒绝了这个请求（HTTP ${entry.status}）`;
    body = (
      <>
        {tags.some((t) => t.startsWith("整流未生效")) && "ModelLink 自动修复过一次，没能救回来。"}
        {detail || "服务商没有说明原因。"}
      </>
    );
  }

  return (
    <div className="relative border-b border-hair bg-sunken before:absolute before:inset-y-0 before:left-0 before:w-[2px] before:bg-danger">
      <div className="py-3 pr-[22px] pb-3.5 pl-[114px] text-[12.5px] leading-[1.7] text-fg2">
        <b className="mb-0.5 block font-semibold text-fg">{title}</b>
        {body}
        {action && (
          <button
            onClick={action.run}
            className="mt-2 flex h-7 items-center rounded-[7px] bg-panel px-[11px] text-[12px] text-fg shadow-[inset_0_0_0_1px_var(--hair2)] transition-colors hover:bg-hair"
          >
            {action.label}
          </button>
        )}
      </div>
    </div>
  );
}

/** 请求日志页（design-2.2.md §6.3）：今日摘要 + 筛选 + 数据行，失败行摊开原因。 */
export function LogsPage() {
  const { draft } = useAppStore();
  const qc = useQueryClient();
  const logsQuery = useQuery({ queryKey: ["logs"], queryFn: getLogs, refetchInterval: 2000 });
  const statsQuery = useQuery({ queryKey: ["log-stats"], queryFn: getLogStats, refetchInterval: 2000 });
  const [filter, setFilter] = useState<Filter>("all");

  const entries = [...(logsQuery.data ?? [])].reverse();
  const showCost = pricingComplete(draft);

  const failedCount = entries.filter(isFailure).length;
  const rectifiedCount = entries.filter(isRectified).length;
  // 按模型筛：只列最常见的三个，免得挤满一行
  const byModel = new Map<string, number>();
  for (const e of entries) byModel.set(e.model, (byModel.get(e.model) ?? 0) + 1);
  const topModels = [...byModel.entries()].sort((a, b) => b[1] - a[1]).slice(0, 3);

  const shown = entries.filter((e) =>
    filter === "all"
      ? true
      : filter === "failed"
        ? isFailure(e)
        : filter === "rectified"
          ? isRectified(e)
          : e.model === filter.slice("model:".length),
  );

  const chip = (key: Filter, label: string, count: number, danger = false) => {
    const on = filter === key;
    return (
      <button
        key={key}
        onClick={() => setFilter(on && key !== "all" ? "all" : key)}
        className={cn(
          "flex h-[30px] max-w-[220px] flex-none items-center gap-[7px] rounded-ctl px-[13px] text-[12.5px] text-fg2 inset-ring inset-ring-hair2 transition-colors hover:text-fg",
          on && !danger && "bg-accent-weak font-semibold text-accent inset-ring-[1.5px] inset-ring-accent hover:text-accent",
          on && danger && "bg-danger/7 font-semibold text-danger inset-ring-[1.5px] inset-ring-danger hover:text-danger",
        )}
      >
        <span className="truncate">{label}</span>
        <span className={cn("mono text-[11.5px] text-fg3", on && "text-current opacity-65")}>{count}</span>
      </button>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="请求日志"
        sub={
          <>
            保留最近 <span className="mono">100</span> 条 · 停留在本页时每 <span className="mono">2</span> 秒自动刷新
          </>
        }
        right={
          <Button
            variant="ghost"
            onClick={() => {
              void qc.invalidateQueries({ queryKey: ["logs"] });
              void qc.invalidateQueries({ queryKey: ["log-stats"] });
            }}
          >
            <RefreshCw className={cn(logsQuery.isFetching && "animate-spin")} />
            刷新
          </Button>
        }
      />

      <Summary stats={statsQuery.data} showCost={showCost} />

      <div className="mb-3 flex flex-none items-center gap-1.5 overflow-hidden">
        {chip("all", "全部", entries.length)}
        {chip("failed", "仅失败", failedCount, true)}
        {chip("rectified", "已自动修复", rectifiedCount)}
        {topModels.map(([m, n]) => chip(`model:${m}`, m, n))}
      </div>

      <div className="panel mb-5 flex min-h-0 flex-1 flex-col">
        <div className="flex h-[34px] flex-none items-center gap-3 border-b border-hair px-[22px] text-label text-fg3">
          <span className={COL.time}>时间</span>
          <span className={COL.dot} />
          <span className={COL.model}>模型</span>
          <span className={COL.ms}>耗时</span>
          <span className={COL.tokens}>TOKEN 入 → 出</span>
          {showCost && <span className={COL.cost}>花费</span>}
          <span className={COL.code}>状态</span>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {shown.length === 0 && (
            <div className="px-[22px] py-10 text-center text-[12.5px] leading-[1.7] text-fg3">
              {entries.length === 0 ? (
                <>
                  还没有请求记录。Claude Desktop 发出的每个请求都会出现在这里。
                  <br />
                  如果 Claude 里一直连不上、这里却一条都没有，先去概览页看看哪一环没通。
                </>
              ) : (
                "这个筛选下没有记录"
              )}
            </div>
          )}

          {shown.map((e) => {
            const failed = isFailure(e);
            return (
              <Fragment key={e.id}>
                <div className="flex h-11 items-center gap-3 border-b border-hair px-[22px]">
                  <span className={cn(COL.time, "mono text-[12px] text-fg3")}>{e.time}</span>
                  <i className={cn(COL.dot, "h-1.5 rounded-full", failed ? "bg-danger" : "bg-ok")} />
                  <span className={COL.model}>
                    <span className="mono min-w-0 truncate text-[13px] font-medium tracking-[-0.01em]">
                      {e.model}
                    </span>
                    <span className="flex-1" />
                    {THINKING_TAGS[e.thinking] && e.thinking && (
                      <span className="flex-none rounded-sm px-2 py-0.5 text-[11.5px] text-fg2 inset-ring inset-ring-hair2">
                        {THINKING_TAGS[e.thinking]}
                      </span>
                    )}
                    {noteTags(e.note).map((t) => (
                      <span
                        key={t.text}
                        className={cn(
                          "flex-none rounded-sm px-2 py-0.5 text-[11.5px] inset-ring",
                          t.tone === "ok" && "text-ok inset-ring-ok/30",
                          t.tone === "bad" && "text-danger inset-ring-danger/30",
                          t.tone === "neutral" && "text-fg2 inset-ring-hair2",
                        )}
                      >
                        {t.text}
                      </span>
                    ))}
                  </span>
                  <span className={cn(COL.ms, "mono text-[12px] text-fg2")}>
                    {e.duration_ms === null ? <span className="text-fg3">传输中</span> : formatDuration(e.duration_ms)}
                  </span>
                  <span className={cn(COL.tokens, "mono text-[12px] text-fg2")}>
                    {e.usage ? `${formatTokens(promptTokens(e.usage))} → ${formatTokens(e.usage.output_tokens)}` : "—"}
                  </span>
                  {showCost && (
                    <span className={cn(COL.cost, "mono text-[12px] text-fg2")}>
                      {e.cost_usd === null ? "—" : formatUsd(e.cost_usd)}
                    </span>
                  )}
                  <span className={cn(COL.code, "mono text-[12.5px] font-semibold", failed ? "text-danger" : "text-ok")}>
                    {e.status}
                  </span>
                </div>
                {failed && <FailureNote entry={e} />}
              </Fragment>
            );
          })}
        </div>
      </div>
    </div>
  );
}
