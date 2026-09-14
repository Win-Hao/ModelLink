import { AnimatePresence, motion } from "framer-motion";
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle,
  Download,
  RotateCw,
  Sparkles,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import type { UpdateState } from "@/lib/useUpdater";

type Props = {
  open: boolean;
  state: UpdateState;
  onUpdate: () => void; // 立即更新（下载+安装+重启）
  onLater: () => void; // 稍后（关闭 + 进入 24h 冷却）
  onSkip: () => void; // 跳过此版本
};

/**
 * 发现新版本弹窗（样式套 2.2 设计 token）。
 * 启动静默检查到更新且未被「跳过 / 冷却」则浮现。
 */
export function UpdateModal({ open, state, onUpdate, onLater, onSkip }: Props) {
  const busy = state.isDownloading || state.isInstalling || state.isRestarting;
  const manualRestart = state.requiresManualRestart;

  const primaryLabel = state.isDownloading
    ? `下载中 ${state.downloadProgress}%`
    : state.isInstalling
      ? "安装中…"
      : state.isRestarting
        ? "重启中…"
        : "立即更新";

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-50 flex items-center justify-center p-6"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
        >
          <button
            className="absolute inset-0 bg-[rgba(20,17,14,.32)] dark:bg-black/60"
            aria-label="稍后"
            onClick={busy ? undefined : onLater}
          />
          <motion.div
            className="relative flex w-full max-w-md flex-col overflow-hidden rounded-lg bg-panel text-fg shadow-float"
            initial={{ opacity: 0, y: 10, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.98 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
          >
            {/* 头部 */}
            <div className="flex items-start justify-between gap-3 px-5 pb-3 pt-5">
              <div className="flex items-center gap-2.5">
                <span className="grid size-9 shrink-0 place-items-center rounded-full bg-accent-weak text-accent">
                  <Sparkles size={18} />
                </span>
                <div>
                  <div className="text-heading">发现新版本</div>
                  <div className="mono text-[12px] text-fg3">ModelLink v{state.newVersion ?? "?"}</div>
                </div>
              </div>
              {!busy && (
                <button
                  className="flex size-[26px] items-center justify-center rounded-[7px] text-fg3 transition-colors hover:bg-hair hover:text-fg"
                  onClick={onLater}
                  aria-label="稍后"
                >
                  <X size={14} />
                </button>
              )}
            </div>

            {/* 版本对比 */}
            <div className="mx-5 mb-3 flex items-center justify-center gap-4 rounded-md bg-sunken py-3 inset-ring inset-ring-hair">
              <div className="text-center">
                <div className="text-label text-fg3">当前</div>
                <div className="mono text-sm font-medium">{state.currentVersion || "—"}</div>
              </div>
              <ArrowRight size={16} className="text-fg3" />
              <div className="text-center">
                <div className="text-label text-fg3">最新</div>
                <div className="mono text-sm font-semibold text-fg">{state.newVersion ?? "—"}</div>
              </div>
            </div>

            {/* 更新说明 */}
            {state.notes?.trim() && (
              <div className="mx-5 mb-4">
                <div className="mb-1.5 text-label text-fg3">更新内容</div>
                <div className="max-h-32 overflow-y-auto rounded-md bg-sunken px-3 py-2.5 text-[12px] leading-relaxed whitespace-pre-wrap text-fg2 inset-ring inset-ring-hair">
                  {state.notes.trim()}
                </div>
              </div>
            )}

            {/* 下载进度 */}
            {state.isDownloading && (
              <div className="mx-5 mb-4 space-y-1.5">
                <div className="flex items-center gap-2 text-[12px] text-fg2">
                  <Download size={13} />
                  下载中 {state.downloadProgress}%
                </div>
                <Progress value={state.downloadProgress} className="h-1.5" />
              </div>
            )}

            {/* 安装中 / 重启中 */}
            {(state.isInstalling || state.isRestarting) && !manualRestart && (
              <div className="mx-5 mb-4 flex items-center gap-2 text-[12px] text-fg2">
                <RotateCw size={13} className="animate-spin text-fg3" />
                {state.isRestarting ? "安装完成，正在重启…" : "安装中…"}
              </div>
            )}

            {/* 已下载但需手动重启 */}
            {manualRestart && (
              <div className="mx-5 mb-4 rounded-md px-3 py-2.5 inset-ring inset-ring-ok/30">
                <div className="flex items-center gap-2 text-[12px] font-medium text-ok">
                  <CheckCircle size={14} className="shrink-0" />
                  更新已下载完成
                </div>
                <p className="mt-1 pl-6 text-[11.5px] text-fg3">
                  请手动退出 ModelLink（⌘Q）后重新打开即可用上新版本。
                </p>
              </div>
            )}

            {/* 出错 */}
            {state.error && !busy && !manualRestart && (
              <div className="mx-5 mb-4 rounded-md px-3 py-2.5 inset-ring inset-ring-danger/30">
                <div className="flex items-center gap-2 text-[12px] text-danger">
                  <AlertTriangle size={14} className="shrink-0" />
                  更新出错：{state.error}
                </div>
                <p className="mt-1 text-[11.5px] text-fg3">
                  可稍后重试，或到设置页前往 GitHub 手动下载。
                </p>
              </div>
            )}

            {/* 操作 */}
            <div className="flex flex-col gap-2 px-5 pb-5">
              {manualRestart ? (
                <Button className="w-full" onClick={onLater}>
                  知道了
                </Button>
              ) : (
                <>
                  <Button className="w-full disabled:opacity-80" onClick={onUpdate} disabled={busy}>
                    {busy ? <RotateCw className="animate-spin" /> : <Download />}
                    {primaryLabel}
                  </Button>
                  {!busy && (
                    <div className="flex items-center justify-center gap-4 text-[12px]">
                      <button className="text-fg3 transition-colors hover:text-fg" onClick={onLater}>
                        稍后提醒
                      </button>
                      <span className="text-hair2">·</span>
                      <button className="text-fg3 transition-colors hover:text-fg" onClick={onSkip}>
                        跳过此版本
                      </button>
                    </div>
                  )}
                </>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
