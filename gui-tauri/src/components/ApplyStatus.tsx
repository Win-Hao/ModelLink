import type { ReactNode } from "react";
import { Check, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAppStore } from "@/lib/store";

/**
 * 页头右侧的应用状态（design-2.2.md §7）。概览页与服务商页用的是同一个，
 * 状态差在哪一页都指向同一个动作。
 *
 * 上面一行说明、下面一个按钮；applying 态也保留那一行，免得页头高度跳一下。
 */
export function ApplyStatusArea() {
  const { applyState, applyError, apply, poolUpgrade, configChanged, pendingCount } = useAppStore();

  let line: ReactNode;
  let action: ReactNode;
  switch (applyState) {
    case "clean":
      // 没事的时候界面是安静的：强调色一次都不用
      line = (
        <span className="flex items-center justify-end gap-1.5 text-ok">
          <Check size={12} strokeWidth={2.4} />
          配置已生效
        </span>
      );
      action = (
        <Button variant="ghost" onClick={apply}>
          重新应用
        </Button>
      );
      break;
    case "dirty":
      line = (
        <span className="text-accent">
          {poolUpgrade
            ? // 升级换了槽位池：老用户得知道为什么突然又要应用一次
              "新版为你的模型启用了 Claude Desktop 原生推理强度选择器"
            : !configChanged
              ? // 这里没改，是 Claude 那边的配置变了（被别的工具或手动改过）
                `Claude Desktop 里有 ${pendingCount} 处和这里对不上`
              : pendingCount > 0
                ? `有 ${pendingCount} 处改动尚未应用`
                : "配置已修改，尚未应用"}
        </span>
      );
      action = <Button onClick={apply}>应用到 Claude Desktop</Button>;
      break;
    case "applying":
      line = <span className="text-fg3">Claude Desktop 会自动重启</span>;
      action = (
        <Button disabled className="disabled:opacity-80">
          <Loader2 className="animate-spin" />
          正在重启 Claude…
        </Button>
      );
      break;
    case "error":
      line = (
        <span className="block max-w-[360px] truncate text-danger" title={applyError ?? undefined}>
          应用失败：{applyError}
        </span>
      );
      action = <Button onClick={apply}>重试</Button>;
      break;
  }

  return (
    <div className="flex flex-col items-end gap-[9px]">
      <div className="text-[12.5px] leading-[1.5] tracking-[-0.003em]">{line}</div>
      {action}
    </div>
  );
}
