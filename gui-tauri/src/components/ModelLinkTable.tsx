import { useRef } from "react";
import { motion } from "framer-motion";

import { ProviderAvatar, brandForUrl } from "@/components/ProviderAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import {
  MAX_MODELS,
  claims1mItDoesNotHave,
  flattenModels,
  formatContext,
  providerDisplayName,
} from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { useTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

// 列宽（design-2.2.md §6.1）：槽位 168 · 箭头 26 · 模型 flex · 上下文 78 · 推理强度 104 · 状态 96
const COL = {
  slot: "w-[168px] flex-none",
  arrow: "flex w-[26px] flex-none",
  model: "flex min-w-0 flex-1 items-center gap-[11px]",
  ctx: "w-[78px] flex-none text-right",
  effort: "w-[104px] flex-none text-right",
  state: "w-[96px] flex-none text-right",
};

/** 箭头：未应用时变虚线 —— 这条映射还没接通到 Claude。 */
function LinkArrow({ dashed }: { dashed: boolean }) {
  return (
    <svg width="13" height="13" viewBox="0 0 14 14" fill="none" aria-hidden>
      <path
        d="M2.5 7h9m-3.3-3.3L11.5 7l-3.3 3.3"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeDasharray={dashed ? "2.6 2.6" : undefined}
      />
    </svg>
  );
}

/**
 * 模型链路表（概览页唯一主体）：Claude 里的每个模型名 → 实际接入的模型。
 *
 * 扁平、按槽位顺序排，不分组 —— 同一服务商的模型天然连续占槽（展开逻辑是
 * 外层遍历服务商、内层遍历模型）。空槽不逐行列出，面板底部一行带过。
 */
export function ModelLinkTable() {
  const { draft, gotoProvider, flashNonce, appliedKnown, pendingSlots } = useAppStore();
  const { dark } = useTheme();
  // 只闪「这次停在本页时」发生的应用；切页回来不重放
  const flashFrom = useRef(flashNonce);

  const rows = draft ? flattenModels(draft) : [];
  const free = MAX_MODELS - rows.length;

  return (
    <div className="panel flex min-h-0 flex-col">
      <div className="flex h-[38px] flex-none items-center border-b border-hair px-[22px] text-label tracking-[0.06em] text-fg3">
        <span className={COL.slot}>CLAUDE 槽位</span>
        <span className={COL.arrow} />
        <span className={COL.model}>实际接入的模型</span>
        <span className={COL.ctx}>上下文</span>
        <span className={COL.effort}>推理强度</span>
        <span className={COL.state}>状态</span>
      </div>

      <div className="min-h-0 overflow-y-auto">
        {!draft &&
          [0, 1, 2].map((i) => (
            <div key={i} className="flex h-[52px] items-center border-b border-hair px-[22px] last:border-b-0">
              <Skeleton className="h-4 w-full" />
            </div>
          ))}

        {rows.map((row, i) => {
          const provider = draft!.providers[row.providerIndex];
          const model = provider.models[row.modelIndex];
          const name = providerDisplayName(provider.target_url, row.providerIndex);
          const pending = appliedKnown && pendingSlots.has(row.slot);
          // 开着 1M 但已知装不下：用户以为有 1M，实际没有 —— 标出真实上限
          const bad1m = claims1mItDoesNotHave(model);

          return (
            <button
              key={row.slot}
              onClick={() => gotoProvider(row.providerIndex)}
              className={cn(
                "relative flex h-[52px] w-full items-center border-b border-hair px-[22px] text-left transition-colors last:border-b-0 hover:bg-sunken",
                pending && "before:absolute before:inset-y-0 before:left-0 before:w-[2px] before:bg-accent",
              )}
            >
              {/* apply 成功：各行依次绿色微闪（唯一表演性动效） */}
              {flashNonce > flashFrom.current && (
                <motion.span
                  key={flashNonce}
                  aria-hidden
                  className="pointer-events-none absolute inset-0"
                  style={{ background: dark ? "rgba(102,192,147,.14)" : "rgba(35,119,88,.1)" }}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: [0, 1, 0] }}
                  transition={{ duration: 0.3, delay: i * 0.06 }}
                />
              )}
              <span className={cn(COL.slot, "mono truncate text-[12.5px]", pending ? "text-fg3" : "text-fg2")}>
                {row.slot}
              </span>
              <span className={cn(COL.arrow, "text-hair2 dark:text-white/15")}>
                <LinkArrow dashed={pending} />
              </span>
              <span className={COL.model}>
                <ProviderAvatar
                  brand={brandForUrl(provider.target_url)}
                  letter={name[0]}
                  className={cn(pending && "opacity-50")}
                />
                <span className={cn("mono truncate text-data", pending && "text-fg3")}>{row.name}</span>
                <span className="flex-none text-[12.5px] tracking-[-0.003em] text-fg3">{name}</span>
              </span>
              <span
                className={cn(COL.ctx, "mono text-[12.5px]", bad1m ? "text-danger" : "text-fg2")}
                title={
                  bad1m
                    ? `开着 1M 变体，但这个模型最多只有 ${formatContext(model.context_limit)}`
                    : undefined
                }
              >
                {formatContext(model.context_limit) || <span className="text-fg3">—</span>}
              </span>
              <span
                className={cn(COL.effort, "text-[12.5px] text-fg2")}
                title={
                  row.efforts.length > 0
                    ? `Claude 里可选 ${row.efforts.join(" / ")}`
                    : "这个槽位在 Claude 里没有推理强度选择器"
                }
              >
                {row.efforts.length > 0 ? `${row.efforts.length} 档` : "无"}
              </span>
              <span className={cn(COL.state, "text-[12.5px]", pending ? "text-accent" : "text-fg2")}>
                {appliedKnown && (
                  <>
                    <i
                      className={cn(
                        "mr-[7px] inline-block size-[5px] rounded-full align-middle",
                        pending ? "bg-accent" : "bg-ok",
                      )}
                    />
                    {pending ? "未应用" : "已生效"}
                  </>
                )}
              </span>
            </button>
          );
        })}
      </div>

      {draft && free > 0 && (
        <div className="flex h-11 flex-none items-center border-t border-hair px-[22px] text-[12.5px] text-fg3">
          <span className="mono mr-1.5 font-semibold text-fg2">{free}</span>
          个槽位空闲 · Claude 里请求这些名字会直接报错，不会静默换成别的模型
        </div>
      )}
    </div>
  );
}
