import { useQuery } from "@tanstack/react-query";

import { getLogs } from "@/lib/ipc";
import { useAppStore } from "@/lib/store";
import { THINKING_TAGS } from "@/lib/presets";
import { cn } from "@/lib/utils";

/** 概览页「最近请求」卡：最新 2 条 + 查看全部（design.md §6.1）。 */
export function RecentRequests() {
  const { setPage } = useAppStore();
  const { data: logs } = useQuery({
    queryKey: ["logs"],
    queryFn: getLogs,
    refetchInterval: 2000,
  });

  const latest = (logs ?? []).slice(-2).reverse();

  return (
    <div className="rounded-xl border bg-card">
      <div className="flex items-baseline justify-between px-4 pt-[13px]">
        <span className="text-[11px] font-semibold tracking-[.08em] text-faint">最近请求</span>
        <button
          onClick={() => setPage("logs")}
          className="mono text-[11px] text-faint transition-colors hover:text-muted-foreground"
        >
          查看全部 →
        </button>
      </div>
      <div className="flex flex-col px-2 pb-2 pt-1">
        {latest.length === 0 && (
          <div className="px-2 py-[9px] text-[11.5px] text-faint">暂无请求记录</div>
        )}
        {latest.map((l, i) => (
          <div
            key={`${l.time}-${i}`}
            className={cn(
              "flex items-center gap-3 rounded-[8px] px-2 py-[7px]",
              i > 0 && "border-t",
            )}
          >
            <span className="mono w-[70px] flex-none truncate text-[11.5px] text-faint">
              {l.time}
            </span>
            <span
              className={cn(
                "size-1.5 flex-none rounded-full",
                l.status === 200 && !l.error ? "bg-success" : "bg-destructive",
              )}
            />
            <span className="flex min-w-0 flex-1 items-center">
              <span className="mono truncate text-xs font-medium text-foreground">{l.model}</span>
            </span>
            {THINKING_TAGS[l.thinking] && (
              <span className="flex-none rounded-[5px] border bg-background px-1.5 py-px text-[10px] text-muted-foreground">
                {THINKING_TAGS[l.thinking]}
              </span>
            )}
            {l.note && (
              <span
                className={cn(
                  "flex-none rounded-[5px] border px-1.5 py-px text-[10px]",
                  l.error
                    ? "border-destructive/30 bg-destructive-soft text-destructive"
                    : "border-success/30 bg-success-soft text-success",
                )}
              >
                {l.note}
              </span>
            )}
            <span
              className={cn(
                "mono flex-none text-[11px]",
                l.status === 200 && !l.error ? "text-success" : "text-destructive",
              )}
            >
              {l.status}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
