import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { GITHUB_URL } from "@/lib/constants";
import { KEY_FEATURE_NAMES, formatAppliedAt } from "@/lib/presets";
import { desktopInfo, guiVersion, proxyStatus, syncPricing } from "@/lib/ipc";
import { useAppStore } from "@/lib/store";
import { useTheme, type ThemePref } from "@/lib/theme";
import { useUpdaterCtx } from "@/lib/updaterContext";

const THEME_TABS: { value: ThemePref; label: string }[] = [
  { value: "light", label: "亮色" },
  { value: "dark", label: "深色" },
  { value: "system", label: "跟随系统" },
];

/** 设置页（design.md §6.4）：外观 / 代理端口 / 兼容模式 / 开机自启 / 软件更新 / 关于。 */
export function SettingsPage() {
  const { pref, setPref } = useTheme();
  const { changePort, draft, updateDraft, reloadConfig } = useAppStore();
  const updater = useUpdaterCtx();
  const qc = useQueryClient();

  const versionQ = useQuery({ queryKey: ["gui-version"], queryFn: guiVersion });
  const autostartQ = useQuery({ queryKey: ["autostart"], queryFn: () => isEnabled() });
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });

  // 端口输入（本地编辑态，blur/Enter 提交热切换）
  const [portText, setPortText] = useState("");
  const [switching, setSwitching] = useState(false);
  useEffect(() => {
    if (statusQ.data) setPortText(String(statusQ.data.port));
  }, [statusQ.data]);

  const submitPort = async () => {
    const cur = statusQ.data?.port;
    const n = Number(portText);
    if (!portText || n === cur) {
      setPortText(cur ? String(cur) : "");
      return;
    }
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      toast.error("端口需在 1024–65535 之间");
      setPortText(cur ? String(cur) : "");
      return;
    }
    setSwitching(true);
    try {
      await changePort(n);
    } catch (e) {
      toast.error(`切换端口失败：${String(e)}`);
      setPortText(cur ? String(cur) : "");
    }
    setSwitching(false);
  };

  const desktopQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });
  const unavailable = (desktopQ.data?.unavailable ?? []).map(
    (k) => KEY_FEATURE_NAMES[k] ?? k,
  );

  // 手动同步费率：无视自动开关与 6 小时阈值
  const [syncing, setSyncing] = useState(false);
  const runSync = async () => {
    setSyncing(true);
    try {
      const r = await syncPricing(true);
      if (!r.ok) toast.error(`费率同步失败：${r.message}`);
      else if (r.changed > 0) toast.success(`费率已更新：${r.changed} 个模型`);
      else toast.success("费率已是最新");
      await reloadConfig();
    } catch (e) {
      toast.error(`费率同步失败：${String(e)}`);
    }
    setSyncing(false);
  };

  const toggleAutostart = async (ck: boolean) => {
    try {
      if (ck) await enable();
      else await disable();
    } catch (e) {
      toast.error(`设置开机自启失败：${String(e)}`);
    }
    await qc.invalidateQueries({ queryKey: ["autostart"] });
  };

  const version = versionQ.data ?? "";
  const updateSub = updater.state.hasUpdate
    ? `当前 ${version} · 发现新版本 v${updater.state.newVersion}`
    : updater.checked
      ? `当前 ${version} · 已是最新版本`
      : `当前 ${version}`;

  return (
    <>
      <header className="flex items-end justify-between gap-3.5 px-6 pb-3.5 pt-[46px]">
        <div>
          <h1 className="text-[19px] font-[650] leading-[1.25] tracking-[-0.01em]">设置</h1>
        </div>
      </header>

      <div className="flex flex-1 flex-col overflow-y-auto px-6 pb-6 pt-0.5">
        <div className="rounded-xl border bg-card">
          {/* 外观 */}
          <div className="flex items-center justify-between px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">外观</div>
            </div>
            <Tabs value={pref} onValueChange={(v) => setPref(v as ThemePref)}>
              <TabsList className="h-auto gap-[2px] rounded-[9px] border bg-background p-[2px]">
                {THEME_TABS.map((t) => (
                  <TabsTrigger
                    key={t.value}
                    value={t.value}
                    className="rounded-[7px] border-none px-2.5 py-1 text-[11.5px] text-muted-foreground data-[state=active]:bg-card data-[state=active]:font-semibold data-[state=active]:text-foreground data-[state=active]:shadow-[0_1px_3px_rgba(0,0,0,.12)] dark:data-[state=active]:shadow-none"
                  >
                    {t.label}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          </div>

          {/* 代理端口（2026-07-14 用户新增：热切换 + 冲突自救） */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">代理端口</div>
              <div className="mt-px text-[11px] text-faint">
                修改后立即生效，需重新应用到 Claude Desktop
              </div>
            </div>
            <div className="flex items-center gap-2">
              {switching && <Loader2 size={12} className="animate-spin text-faint" />}
              <Input
                value={portText}
                onChange={(e) => setPortText(e.target.value.replace(/[^0-9]/g, ""))}
                onBlur={() => void submitPort()}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                }}
                disabled={switching}
                inputMode="numeric"
                className="mono h-[29px] w-[88px] rounded-[9px] border-input bg-input-bg px-2.5 text-center text-xs md:text-xs shadow-none dark:bg-input-bg"
              />
            </div>
          </div>

          {/* 费率同步（2.1-B §3.1）：数据源 models.dev，社区维护的开源模型数据库 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">自动同步模型费率</div>
              <div className="mt-px text-[11px] text-faint">
                启动时从 models.dev 拉取官方价（最多 6 小时一次）·{" "}
                {draft?.pricing_synced_at
                  ? `上次同步 ${formatAppliedAt(draft.pricing_synced_at) ?? "—"}`
                  : "尚未同步"}
                <br />
                手填的费率不会被覆盖
              </div>
            </div>
            <div className="flex flex-none items-center gap-2">
              <Button
                variant="outline"
                onClick={() => void runSync()}
                disabled={syncing}
                className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
              >
                {syncing && <Loader2 size={12} className="animate-spin" />}
                立即同步
              </Button>
              <Switch
                checked={draft?.pricing_auto_sync ?? true}
                disabled={!draft}
                onCheckedChange={(ck) =>
                  updateDraft((c) => {
                    c.pricing_auto_sync = ck;
                  })
                }
              />
            </div>
          </div>

          {/* 开机自启 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">开机自启</div>
              <div className="mt-px text-[11px] text-faint">登录时自动启动代理</div>
            </div>
            <Switch
              checked={autostartQ.data ?? false}
              onCheckedChange={(ck) => void toggleAutostart(ck)}
            />
          </div>

          {/* 软件更新 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">软件更新</div>
              <div className="mt-px text-[11px] text-faint">
                <span className="mono">{updateSub}</span>
              </div>
            </div>
            <Button
              variant="outline"
              onClick={() => void updater.manualCheck()}
              disabled={updater.state.isChecking}
              className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
            >
              {updater.state.isChecking && <Loader2 size={12} className="animate-spin" />}
              检查更新
            </Button>
          </div>

          {/* 检测到的 Claude Desktop 版本（2.1-E §3.8） */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">Claude Desktop</div>
              <div className="mt-px text-[11px] text-faint">
                {desktopQ.data?.version ? (
                  <>
                    检测到 <span className="mono">{desktopQ.data.version}</span>
                    {unavailable.length > 0 ? (
                      <> · 版本过低，暂不可用：{unavailable.join("、")}</>
                    ) : (
                      <> · 全部能力可用</>
                    )}
                  </>
                ) : (
                  "未检测到安装（写入时不做版本裁剪）"
                )}
              </div>
            </div>
          </div>

          {/* 关于 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">关于</div>
              <div className="mt-px text-[11px] text-faint">
                ModelLink by Winhao学AI · 免费软件 · 不可商业化
              </div>
            </div>
            <button
              onClick={() => void openUrl(GITHUB_URL)}
              className="flex items-center gap-[5px] text-[12.5px] text-muted-foreground transition-colors hover:text-foreground"
            >
              GitHub
              <ExternalLink size={11} />
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
