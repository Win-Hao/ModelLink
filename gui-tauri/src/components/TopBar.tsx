import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, RefreshCw } from "lucide-react";

import appIcon from "@/assets/brand/app-icon.png";
import appIcon2x from "@/assets/brand/app-icon@2x.png";
import appIcon3x from "@/assets/brand/app-icon@3x.png";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { proxyStatus } from "@/lib/ipc";
import { useAppStore, type Page } from "@/lib/store";
import { cn } from "@/lib/utils";

const NAV: { key: Page; label: string }[] = [
  { key: "overview", label: "概览" },
  { key: "providers", label: "服务商" },
  { key: "logs", label: "请求日志" },
  { key: "settings", label: "设置" },
];

/**
 * 顶栏（design-2.2.md §5）：品牌 · 居中胶囊导航 · 代理状态与全局操作。
 * 整条是拖拽区（deep：子元素也能拖），按钮类元素 Tauri 会自动排除。
 */
export function TopBar() {
  const { page, setPage, applyState, reloadConfig, setPickerOpen } = useAppStore();
  const qc = useQueryClient();
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const [refreshing, setRefreshing] = useState(false);

  const refresh = async () => {
    setRefreshing(true);
    try {
      await reloadConfig();
      await qc.invalidateQueries();
    } finally {
      setRefreshing(false);
    }
  };

  // 应用状态差在任何页面都得看得见（PRODUCT.md 原则二），指向概览页的「应用」
  const pending = applyState === "dirty" || applyState === "error";

  return (
    <header data-tauri-drag-region="deep" className="flex h-16 flex-none items-center px-6">
      {/* 三栏等宽：导航始终居中；右侧文案变长（端口被占）时挤开而不是压上导航 */}
      <div className="mx-auto grid w-full max-w-[964px] grid-cols-[1fr_auto_1fr] items-center gap-4">
        <div className="flex items-center gap-2.5 justify-self-start mac:clear-traffic-lights">
          {/* 应用图标本身（branding/icon-source.png 裁掉留白），不随主题换色 */}
          <img
            src={appIcon}
            srcSet={`${appIcon} 1x, ${appIcon2x} 2x, ${appIcon3x} 3x`}
            width={26}
            height={26}
            alt=""
            draggable={false}
            className="flex-none"
          />
          <span className="text-heading tracking-[-0.015em]">ModelLink</span>
        </div>

        <nav
          aria-label="主导航"
          className="flex gap-[2px] rounded-md bg-[rgba(32,28,24,.045)] p-[3px] dark:bg-panel dark:shadow-[inset_0_0_0_1px_var(--hair)]"
        >
          {NAV.map(({ key, label }) => (
            <button
              key={key}
              onClick={() => setPage(key)}
              aria-current={page === key ? "page" : undefined}
              className={cn(
                "relative h-[30px] rounded-[7px] px-3.5 text-[12.5px] font-medium text-fg3 transition-colors outline-none hover:text-fg focus-visible:ring-[3px] focus-visible:ring-ring/40",
                page === key &&
                  "bg-white text-fg shadow-[0_1px_2px_rgba(32,28,24,.09),0_0_0_1px_rgba(32,28,24,.06)] dark:bg-white/6 dark:shadow-[inset_0_0_0_1px_var(--hair2)]",
              )}
            >
              {label}
              {key === "overview" && pending && (
                <span
                  aria-label={applyState === "error" ? "应用失败" : "有改动尚未应用"}
                  className={cn(
                    "absolute top-[5px] right-[5px] size-[5px] rounded-full",
                    applyState === "error" ? "bg-danger" : "bg-accent",
                  )}
                />
              )}
            </button>
          ))}
        </nav>

        <div className="flex items-center gap-[18px] justify-self-end">
          {statusQ.data && (
            <span className="flex items-center gap-[7px] text-[12.5px] whitespace-nowrap text-fg2">
              <span
                className={cn(
                  "size-1.5 rounded-full",
                  statusQ.data.running ? "bg-ok" : "bg-accent",
                )}
              />
              {statusQ.data.running ? "代理运行中" : "代理未运行（端口被占）"}
              <span className="mono text-fg3">127.0.0.1:{statusQ.data.port}</span>
            </span>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label="刷新状态"
                disabled={refreshing || applyState === "applying"}
                onClick={() => void refresh()}
              >
                <RefreshCw className={cn(refreshing && "animate-spin")} />
              </Button>
            </TooltipTrigger>
            <TooltipContent>刷新状态</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label="添加服务商"
                onClick={() => setPickerOpen(true)}
              >
                <Plus />
              </Button>
            </TooltipTrigger>
            <TooltipContent>添加服务商</TooltipContent>
          </Tooltip>
        </div>
      </div>
    </header>
  );
}
