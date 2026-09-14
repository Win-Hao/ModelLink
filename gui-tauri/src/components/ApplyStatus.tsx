import { Check, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useHealth, type PrimaryAction, type Tone } from "@/lib/health";
import { cn } from "@/lib/utils";

const LINE: Record<Tone, string> = {
  ok: "text-ok",
  idle: "text-fg3",
  attention: "text-accent",
  bad: "text-danger",
};

/** 现在最该做的那件事。应用会重启 Claude —— 按钮旁边直说，不等点下去才知道。 */
export function PrimaryActionButton({ primary }: { primary: PrimaryAction | null }) {
  if (!primary) return null;
  if (primary.kind === "busy") {
    return (
      <Button disabled className="disabled:opacity-80">
        <Loader2 className="animate-spin" />
        {primary.label}
      </Button>
    );
  }
  return (
    <div className="flex items-center gap-3">
      {primary.kind === "apply" && <span className="text-[12px] text-fg3">会重启 Claude Desktop</span>}
      <Button variant={primary.kind === "apply" ? "default" : "ghost"} onClick={primary.run}>
        {primary.label}
      </Button>
    </div>
  );
}

/**
 * 页头右侧的应用状态（design-2.2.md §7）：上面一句话、下面一个按钮。
 * 服务商页用；概览页的状态由连通链来说，页头只放按钮。两处读的是同一份 useHealth()。
 */
export function ApplyStatusArea() {
  const { line, primary } = useHealth();
  return (
    // 固定高度：有按钮 / 没按钮之间切换时页头不跳，没事时那句话贴着底和副标题对齐
    <div className="flex min-h-[66px] flex-col items-end justify-end gap-[9px]">
      <div
        className={cn("flex max-w-[420px] items-center gap-1.5 text-[12.5px] leading-[1.5] tracking-[-0.003em]", LINE[line.tone])}
        title={line.text}
      >
        {line.tone === "ok" && <Check size={12} strokeWidth={2.4} className="flex-none" />}
        <span className="truncate">{line.text}</span>
      </div>
      {/* 没事做时不放按钮 —— 页头保持安静 */}
      <PrimaryActionButton primary={primary} />
    </div>
  );
}
