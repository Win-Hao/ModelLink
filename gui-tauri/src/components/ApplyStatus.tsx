import { Check, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAppStore } from "@/lib/store";

// 应用状态机的三处同源 UI 之一/之二（design.md §8）：
// 概览页头右侧状态区 + 服务商页头「应用」胶囊。第三处是侧栏琥珀点（Sidebar 内）。

/** 概览页头右侧：clean / dirty / applying / error 四态。 */
export function ApplyStatusArea() {
  const { applyState, applyError, apply, poolUpgrade } = useAppStore();

  return (
    <div className="flex flex-none flex-col items-end gap-1.5">
      {applyState === "clean" && (
        <>
          <span className="flex items-center gap-[5px] text-[11.5px] font-medium text-success">
            <Check size={12} strokeWidth={2.2} />
            配置已生效
          </span>
          <Button
            variant="outline"
            onClick={apply}
            className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
          >
            重新应用
          </Button>
        </>
      )}

      {applyState === "dirty" && (
        <>
          <span className="flex items-center gap-1.5 text-[11.5px] font-medium text-warning">
            <span className="size-1.5 rounded-full bg-current" />
            {/* §2.2：升级换了槽位池，老用户得知道为什么突然又要应用一次 */}
            {poolUpgrade
              ? "新版为你的模型启用了 Claude Desktop 原生推理强度选择器"
              : "配置已修改，尚未应用"}
          </span>
          <Button
            onClick={apply}
            className="h-[35px] rounded-[9px] px-4 text-[13px] font-medium hover:bg-primary-hover"
          >
            应用到 Claude Desktop
          </Button>
        </>
      )}

      {applyState === "applying" && (
        <Button disabled className="h-[35px] rounded-[9px] px-4 text-[13px] font-medium">
          <Loader2 size={14} className="animate-spin" />
          正在重启 Claude…
        </Button>
      )}

      {applyState === "error" && (
        <>
          <span
            className="max-w-[320px] truncate text-[11.5px] font-medium text-destructive"
            title={applyError ?? undefined}
          >
            {applyError}
          </span>
          <Button
            onClick={apply}
            className="h-[35px] rounded-[9px] px-4 text-[13px] font-medium hover:bg-primary-hover"
          >
            重试
          </Button>
        </>
      )}
    </div>
  );
}

/** 服务商页头「应用」胶囊：dirty/applying/error 时出现，点击即 apply（不必回概览页）。 */
export function ApplyPill() {
  const { applyState, apply, poolUpgrade } = useAppStore();
  if (applyState === "clean") return null;

  return (
    <span className="inline-flex flex-none items-center gap-2 rounded-full bg-warning-soft py-[5px] pl-3 pr-1.5 text-[11.5px] font-semibold text-warning">
      {poolUpgrade && applyState === "dirty" ? "新版槽位待启用" : "有未应用的更改"}
      <button
        onClick={apply}
        disabled={applyState === "applying"}
        className="rounded-full bg-primary px-2.5 py-[3px] text-[11px] font-medium text-primary-foreground transition-colors hover:bg-primary-hover disabled:opacity-60"
      >
        {applyState === "applying" ? "应用中…" : "应用"}
      </button>
    </span>
  );
}
