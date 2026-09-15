import { Check, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useHealth, type ApplyAction, type Tone } from "@/lib/health";
import { cn } from "@/lib/utils";

const LINE: Record<Tone, string> = {
  ok: "text-ok",
  idle: "text-fg3",
  attention: "text-accent",
  bad: "text-danger",
};

/**
 * 页头的「应用」按钮（design-2.2.md §7）：一直在，轻重跟着状态走。
 * 应用会重启 Claude —— 按钮旁边直说，不等点下去才知道；点不了的时候，旁边写先做什么。
 */
export function ApplyButton({ action, withReason = true }: { action: ApplyAction | null; withReason?: boolean }) {
  if (!action) return null;
  if (action.kind === "busy") {
    return (
      <Button disabled className="disabled:opacity-80">
        <Loader2 className="animate-spin" />
        {action.label}
      </Button>
    );
  }
  const aside = action.kind === "blocked" ? (withReason ? action.reason : null) : action.note;
  return (
    <div className="flex items-center gap-3">
      {aside && <span className="text-[12px] text-fg3">{aside}</span>}
      {action.kind === "blocked" ? (
        <Button variant="ghost" disabled>
          {action.label}
        </Button>
      ) : (
        <Button variant={action.tone === "primary" ? "default" : "ghost"} onClick={action.run}>
          {action.label}
        </Button>
      )}
    </div>
  );
}

/**
 * 服务商页页头右侧（design-2.2.md §7）：上面一句话，下面修的按钮 + 应用按钮。
 * 概览页的状态由连通链来说，页头只放应用按钮。两处读的是同一份 useHealth()。
 */
export function ApplyStatusArea() {
  const { line, apply, fix } = useHealth();
  return (
    // 固定高度：按钮轻重切换时页头不跳，那句话贴着底和副标题对齐
    <div className="flex min-h-[66px] flex-col items-end justify-end gap-[9px]">
      <div
        className={cn("flex max-w-[420px] items-center gap-1.5 text-[12.5px] leading-[1.5] tracking-[-0.003em]", LINE[line.tone])}
        title={line.text}
      >
        {line.tone === "ok" && <Check size={12} strokeWidth={2.4} className="flex-none" />}
        <span className="truncate">{line.text}</span>
      </div>
      {/* 修的按钮和应用按钮之间比「会重启 Claude」和它的按钮之间离得远，那句话才不会被看成是说修的按钮 */}
      <div className="flex items-center gap-5">
        {fix && (
          <Button variant="ghost" onClick={fix.run}>
            {fix.label}
          </Button>
        )}
        {/* 缺什么上面那句话已经说了，按钮旁边不再说一遍 */}
        <ApplyButton action={apply} withReason={false} />
      </div>
    </div>
  );
}
