import { useEffect, useState } from "react";
import { AnimatePresence, MotionConfig, motion } from "framer-motion";
import { toast } from "sonner";

import { ProviderPickerDialog } from "@/components/ProviderPickerDialog";
import { TopBar } from "@/components/TopBar";
import { UpdateModal } from "@/components/UpdateModal";
import { LogsPage } from "@/components/pages/LogsPage";
import { OverviewPage } from "@/components/pages/OverviewPage";
import { ProvidersPage } from "@/components/pages/ProvidersPage";
import { SettingsPage } from "@/components/pages/SettingsPage";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppStoreProvider, useAppStore } from "@/lib/store";
import { ThemeProvider } from "@/lib/theme";
import { UpdaterCtx } from "@/lib/updaterContext";
import { useUpdater } from "@/lib/useUpdater";

// 更新提醒克制策略（移植 ClaudeCN）：跳过此版本不再自动弹；稍后进入 24h 冷却。
const SKIP_KEY = "modellink.skipUpdateVersion";
const POSTPONE_KEY = "modellink.updatePostponedAt";
const POSTPONE_MS = 24 * 60 * 60 * 1000;
function isUpdateSuppressed(version: string): boolean {
  if (localStorage.getItem(SKIP_KEY) === version) return true;
  const at = Number(localStorage.getItem(POSTPONE_KEY) || 0);
  return at > 0 && Date.now() - at < POSTPONE_MS;
}

export default function App() {
  return (
    <ThemeProvider>
      <TooltipProvider delayDuration={300}>
        <AppStoreProvider>
          <Root />
        </AppStoreProvider>
      </TooltipProvider>
    </ThemeProvider>
  );
}

function Root() {
  const { page } = useAppStore();
  const updater = useUpdater();
  const [updateOpen, setUpdateOpen] = useState(false);
  const [checked, setChecked] = useState(false);

  // 启动静默检查（被「跳过 / 冷却」抑制时不弹）
  useEffect(() => {
    void updater.checkForUpdates().then((u) => {
      setChecked(true);
      if (u && !isUpdateSuppressed(u.version)) setUpdateOpen(true);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const manualCheck = async () => {
    const u = await updater.checkForUpdates();
    setChecked(true);
    if (u) {
      setUpdateOpen(true);
    } else if (!updater.state.error) {
      toast.success("已是最新版本");
    }
  };

  const laterUpdate = () => {
    localStorage.setItem(POSTPONE_KEY, String(Date.now()));
    setUpdateOpen(false);
  };
  const skipUpdate = () => {
    const v = updater.state.newVersion;
    if (v) localStorage.setItem(SKIP_KEY, v);
    setUpdateOpen(false);
  };

  return (
    <MotionConfig reducedMotion="user">
      <UpdaterCtx.Provider value={{ state: updater.state, manualCheck, checked }}>
        {/* 骨架（design-2.2.md §1）：顶栏 64px + 内容栏 964px 居中；窗口缩窄时两侧至少留 24px */}
        <div className="flex h-screen flex-col overflow-hidden bg-bg">
          <TopBar />
          <main className="flex min-h-0 flex-1 flex-col px-6">
            <AnimatePresence mode="wait">
              <motion.div
                key={page}
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 4 }}
                transition={{ duration: 0.16, ease: "easeOut" }}
                className="mx-auto flex min-h-0 w-full max-w-[964px] flex-1 flex-col"
              >
                {page === "overview" && <OverviewPage />}
                {page === "providers" && <ProvidersPage />}
                {page === "logs" && <LogsPage />}
                {page === "settings" && <SettingsPage />}
              </motion.div>
            </AnimatePresence>
          </main>
        </div>

        <ProviderPickerDialog />
        <UpdateModal
          open={updateOpen}
          state={updater.state}
          onUpdate={() => void updater.downloadAndInstall()}
          onLater={laterUpdate}
          onSkip={skipUpdate}
        />
        <Toaster position="bottom-right" />
      </UpdaterCtx.Provider>
    </MotionConfig>
  );
}
