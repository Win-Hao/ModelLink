import { useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { motion } from "framer-motion";

import { ProviderAvatar, brandForUrl } from "@/components/ProviderAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import { desktopInfo, proxyStatus } from "@/lib/ipc";
import { claims1mItDoesNotHave, flattenModels, formatContext, providerDisplayName } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { useTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

// 列宽（design-2.2.md §6.1）：模型 flex · 上下文 78 · 思考深度 112 · 状态 96
const COL = {
  model: "flex min-w-0 flex-1 items-center gap-[11px]",
  ctx: "w-[78px] flex-none text-right",
  effort: "w-[112px] flex-none text-right",
  state: "w-[96px] flex-none text-right",
};

/**
 * 模型表（概览页主体）：接进 Claude 的每个模型。
 *
 * 新版 Claude 的选择器里显示的就是模型真名（apply 写了 labelOverride），所以主列直接写真名；
 * Claude 内部用的槽位名对用户没有意义，只放在悬停里。老版 Claude 不认 labelOverride，
 * 选择器里看到的是槽位名 —— 这时把它摊出来，否则用户在 Claude 里找不到对应的名字。
 *
 * 扁平、按槽位顺序排，不分组 —— 同一服务商的模型天然连续（展开逻辑是外层遍历服务商）。
 */
export function ModelLinkTable() {
  const { draft, gotoProvider, flashNonce, appliedKnown, pendingSlots, pendingApply } = useAppStore();
  const { dark } = useTheme();
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const infoQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });
  // 只闪「这次停在本页时」发生的应用；切页回来不重放
  const flashFrom = useRef(flashNonce);

  const rows = draft ? flattenModels(draft) : [];
  const proxyDown = statusQ.data?.running === false;
  const slotShown = !!infoQ.data?.unavailable.includes("labelOverride");
  // 端口换了还没应用 / Claude 没指向 ModelLink：Claude 眼下连的不是这个代理，哪一行都还没生效
  const allPending = !!pendingApply && (pendingApply.port_changed === true || pendingApply.gateway);

  return (
    <div className="panel flex min-h-0 flex-col">
      <div className="flex h-[38px] flex-none items-center border-b border-hair px-[22px] text-label tracking-[0.06em] text-fg3">
        <span className={COL.model}>模型</span>
        <span className={COL.ctx}>上下文</span>
        <span className={COL.effort}>思考深度</span>
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
          const pending = appliedKnown && (allPending || pendingSlots.has(row.slot));
          // 开着 1M 但已知装不下：用户以为有 1M，实际没有 —— 标出真实上限
          const bad1m = claims1mItDoesNotHave(model);

          return (
            <button
              key={row.slot}
              onClick={() => gotoProvider(row.providerIndex)}
              title={slotShown ? undefined : `Claude 内部用的名字：${row.slot}`}
              aria-label={`${row.name}，${name}。去服务商页编辑`}
              className={cn(
                "relative flex h-[52px] w-full items-center border-b border-hair px-[22px] text-left transition-colors outline-none last:border-b-0 hover:bg-sunken focus-visible:bg-sunken focus-visible:inset-ring-2 focus-visible:inset-ring-ring/40",
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
              <span className={COL.model}>
                <ProviderAvatar
                  brand={brandForUrl(provider.target_url)}
                  letter={name[0]}
                  className={cn(pending && "opacity-60")}
                />
                <span className={cn("mono truncate text-data", pending && "text-fg2")}>{row.name}</span>
                <span className="flex-none text-[12.5px] tracking-[-0.003em] text-fg3">{name}</span>
                {slotShown && (
                  <span className="truncate text-[12px] text-fg3">
                    · Claude 里显示为 <span className="mono text-fg2">{row.slot}</span>
                  </span>
                )}
              </span>
              <span
                className={cn(COL.ctx, "mono text-[12.5px]", bad1m ? "text-danger" : "text-fg2")}
                title={
                  bad1m
                    ? `开着 1M 上下文，但这个模型最多只有 ${formatContext(model.context_limit)}`
                    : undefined
                }
              >
                {formatContext(model.context_limit) || <span className="text-fg3">—</span>}
              </span>
              <span
                className={cn(COL.effort, "text-[12.5px]", row.efforts.length > 0 ? "text-fg2" : "text-fg3")}
                title={
                  row.efforts.length > 0
                    ? `在 Claude 里可以选 ${row.efforts.length} 档思考深度`
                    : "Claude 只给排在前面的 6 个模型提供思考深度选择"
                }
              >
                {row.efforts.length > 0 ? `可调 · ${row.efforts.length} 档` : "不可调"}
              </span>
              <span
                className={cn(
                  COL.state,
                  "text-[12.5px]",
                  proxyDown ? "text-danger" : pending ? "text-accent" : "text-fg2",
                )}
              >
                {appliedKnown && (
                  <>
                    <i
                      className={cn(
                        "mr-[7px] inline-block size-[5px] rounded-full align-middle",
                        proxyDown ? "bg-danger" : pending ? "bg-accent" : "bg-ok",
                      )}
                    />
                    {/* 代理没在跑时，写着「已生效」也用不了 —— 照实说 */}
                    {proxyDown ? "连不上" : pending ? "未应用" : "已生效"}
                  </>
                )}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
