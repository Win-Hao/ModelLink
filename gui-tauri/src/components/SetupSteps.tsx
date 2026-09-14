import { Check } from "lucide-react";

import { cn } from "@/lib/utils";

export type SetupStep = { title: string; sub: string; state: "done" | "current" | "upcoming" | "problem" };

/**
 * 首次配置的三步（design-2.2.md §6.5）：用户是跟着视频走的，任何时候都得知道自己在第几步。
 * 引导页和服务商页（首次应用之前）共用。
 */
export function SetupSteps({ steps, className }: { steps: SetupStep[]; className?: string }) {
  return (
    <ol aria-label="接入步骤" className={cn("flex flex-none", className)}>
      {steps.map((s, i) => (
        <li
          key={s.title}
          aria-current={s.state === "current" || s.state === "problem" ? "step" : undefined}
          className="flex flex-1 items-start gap-[11px] pr-[22px]"
        >
          <span
            className={cn(
              "mono flex size-[22px] flex-none items-center justify-center rounded-full text-[11.5px] font-bold",
              s.state === "done" && "bg-ok/12 text-ok",
              s.state === "current" && "bg-accent-fill text-on-accent",
              s.state === "problem" && "bg-danger-fill text-on-accent",
              s.state === "upcoming" && "text-fg3 inset-ring inset-ring-hair2",
            )}
          >
            {s.state === "done" ? <Check className="size-3" strokeWidth={3} /> : i + 1}
          </span>
          <span className="min-w-0">
            <span
              className={cn(
                "block text-[13px] font-semibold tracking-[-0.01em]",
                s.state === "upcoming" ? "text-fg3" : s.state === "done" ? "text-fg2" : "text-fg",
              )}
            >
              {s.title}
            </span>
            <span
              className={cn(
                "mt-[3px] block text-[12px] leading-[1.5] text-pretty",
                s.state === "problem" ? "text-danger" : "text-fg3",
              )}
            >
              {s.sub}
            </span>
          </span>
        </li>
      ))}
    </ol>
  );
}
