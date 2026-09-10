import { useQuery } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getLogs } from "@/lib/ipc";
import { THINKING_TAGS } from "@/lib/presets";
import { cn } from "@/lib/utils";

/** 请求日志页（design.md §6.3）：整页表格 + 2s 轮询 + 底部诊断提示。 */
export function LogsPage() {
  const logsQuery = useQuery({
    queryKey: ["logs"],
    queryFn: getLogs,
    refetchInterval: 2000,
  });

  const rows = [...(logsQuery.data ?? [])].reverse();

  return (
    <>
      <header className="flex items-end justify-between gap-3.5 px-6 pb-3.5 pt-[46px]">
        <div>
          <h1 className="text-[19px] font-[650] leading-[1.25] tracking-[-0.01em]">请求日志</h1>
          <p className="mt-[3px] text-xs text-muted-foreground">
            保留最近 <span className="mono">100</span> 条 · 页面停留时每{" "}
            <span className="mono">2</span> 秒自动刷新
          </p>
        </div>
        <Button
          variant="outline"
          onClick={() => void logsQuery.refetch()}
          className="h-[29px] flex-none rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
        >
          <RefreshCw size={12} className={cn(logsQuery.isFetching && "animate-spin")} />
          刷新
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col px-6 pb-6 pt-0.5">
        <div className="flex min-h-0 flex-1 flex-col rounded-xl border bg-card">
          <ScrollArea className="min-h-0 flex-1">
            {rows.length === 0 ? (
              <div className="flex h-40 items-center justify-center text-xs text-faint">
                暂无记录
              </div>
            ) : (
              rows.map((l, i) => (
                <div
                  key={`${l.time}-${i}`}
                  className={cn(
                    "mono flex items-center gap-2.5 px-4 py-2 text-[11.5px]",
                    i > 0 && "border-t",
                  )}
                >
                  <span className="flex-none text-faint">{l.time}</span>
                  <span
                    className={cn(
                      "size-1.5 flex-none rounded-full",
                      l.status === 200 && !l.error ? "bg-success" : "bg-destructive",
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate font-semibold text-foreground">
                    {l.model}
                  </span>
                  {THINKING_TAGS[l.thinking] && (
                    <span className="flex-none rounded-[5px] border bg-background px-1.5 py-px font-sans text-[10px] font-normal text-muted-foreground">
                      {THINKING_TAGS[l.thinking]}
                    </span>
                  )}
                  {/* 2.1-A：整流标记 / 未映射槽位等附注 */}
                  {l.note && (
                    <span
                      className={cn(
                        "flex-none rounded-[5px] border px-1.5 py-px font-sans text-[10px] font-normal",
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
                      "flex-none",
                      l.status === 200 && !l.error ? "text-success" : "text-destructive",
                    )}
                  >
                    {l.status}
                  </span>
                </div>
              ))
            )}
          </ScrollArea>
          <div className="border-t px-4 pb-3 pt-2.5 text-[10.5px] text-faint">
            诊断建议：如果这里长期空白而 Claude 无法对话，请检查是否已点「应用到 Claude
            Desktop」。
          </div>
        </div>
      </div>
    </>
  );
}
