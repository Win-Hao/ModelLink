import { motion } from "framer-motion";
import { ArrowRight } from "lucide-react";

import { ProviderAvatar, logoForUrl } from "@/components/ProviderAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import { useAppStore } from "@/lib/store";
import { useTheme } from "@/lib/theme";
import {
  MAX_MODELS,
  flattenModels,
  providerDisplayName,
} from "@/lib/presets";
import { cn } from "@/lib/utils";

/** 概览页主视觉：模型链路板（Claude 槽位 → 真实模型映射，design.md §6.1）。 */
export function LinkBoard() {
  const { draft, gotoProvider, flashNonce } = useAppStore();
  const { dark } = useTheme();

  const flat = draft ? flattenModels(draft) : [];
  const free = MAX_MODELS - flat.length;

  // apply 成功后的绿色微闪（唯一表演性动效，300ms 依次）
  const flashBase = dark ? "77, 184, 160" : "38, 131, 108";
  const flashPeak = dark ? "rgba(77,184,160,0.15)" : "rgba(38,131,108,0.12)";

  return (
    <div className="rounded-xl border bg-card">
      <div className="flex items-baseline justify-between px-4 pt-[13px]">
        <span className="text-[11px] font-semibold tracking-[.08em] text-faint">模型链路</span>
        <span className="mono text-[11px] text-faint">
          {flat.length} / {MAX_MODELS} 槽位
        </span>
      </div>
      <div className="flex flex-col px-2 pb-2.5 pt-1.5">
        {!draft && (
          <div className="flex flex-col gap-2 p-2">
            <Skeleton className="h-[30px] w-full" />
            <Skeleton className="h-[30px] w-full" />
            <Skeleton className="h-[30px] w-full" />
          </div>
        )}

        {draft &&
          flat.map((e, i) => (
            <motion.button
              key={`${flashNonce}-${e.slot}`}
              initial={false}
              animate={
                flashNonce > 0
                  ? {
                      backgroundColor: [
                        `rgba(${flashBase}, 0)`,
                        flashPeak,
                        `rgba(${flashBase}, 0)`,
                      ],
                    }
                  : undefined
              }
              transition={{ duration: 0.3, delay: i * 0.06 }}
              onClick={() => gotoProvider(e.providerIndex)}
              className={cn(
                "flex items-center gap-3 rounded-[8px] px-2 py-[9px] text-left transition-colors hover:bg-background",
                i > 0 && "border-t",
              )}
            >
              <span className="mono w-[150px] flex-none truncate text-[11.5px] text-muted-foreground">
                {e.slot}
              </span>
              {/* §2.1：槽位决定 Claude Desktop 里出不出现推理强度选择器 */}
              <span
                className={cn(
                  "flex w-[58px] flex-none justify-center rounded-[4px] border px-1 py-px text-[9px] font-medium",
                  e.efforts.length > 0
                    ? "border-success/30 text-success"
                    : "border-border text-faint",
                )}
                title={
                  e.efforts.length > 0
                    ? `Claude 里可选 ${e.efforts.join(" / ")}`
                    : "该槽位在 Claude 里没有推理强度选择器"
                }
              >
                {e.efforts.length > 0 ? `${e.efforts.length} 档强度` : "无强度"}
              </span>
              <ArrowRight size={13} className="flex-none text-faint" />
              <span className="flex min-w-0 flex-1 items-center gap-[7px]">
                <span className="mono truncate text-xs font-semibold text-foreground">
                  {e.name}
                </span>
                {e.to1m && (
                  <span className="flex-none rounded-[4px] border border-[rgba(217,119,87,.4)] px-1 text-[9px] font-bold tracking-[.02em] text-primary">
                    1M
                  </span>
                )}
              </span>
              <span className="flex flex-none items-center gap-[5px] rounded-full border bg-background py-[2px] pl-[3px] pr-2 text-[10.5px] text-muted-foreground">
                <ProviderAvatar
                  logo={logoForUrl(draft.providers[e.providerIndex]?.target_url ?? "")}
                  letter={providerDisplayName(draft.providers[e.providerIndex]?.target_url ?? "", e.providerIndex)[0]}
                  size={15}
                />
                {providerDisplayName(draft.providers[e.providerIndex]?.target_url ?? "", e.providerIndex)}
              </span>
            </motion.button>
          ))}

        {draft && flat.length > 0 && free > 0 && (
          <div className="mx-2 mb-0.5 mt-2 rounded-[8px] border border-dashed p-[9px] text-center text-[11.5px] text-faint">
            其余 {free} 个槽位空闲 · 在「服务商」页添加模型后自动接入
          </div>
        )}
      </div>
    </div>
  );
}
