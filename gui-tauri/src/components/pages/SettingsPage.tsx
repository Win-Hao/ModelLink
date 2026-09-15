import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/PageHeader";
import { WinhaoPresetDialog } from "@/components/WinhaoPresetDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { GITHUB_URL } from "@/lib/constants";
import { desktopInfo, guiVersion, proxyStatus, revealClaudeConfig, syncPricing, winhaoPresetState } from "@/lib/ipc";
import { useHealth } from "@/lib/health";
import { KEY_FEATURE_NAMES, formatSince } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { useTheme, type ThemePref } from "@/lib/theme";
import { useUpdaterCtx } from "@/lib/updaterContext";
import { cn } from "@/lib/utils";

const THEME_TABS: { value: ThemePref; label: string }[] = [
  { value: "light", label: "亮色" },
  { value: "dark", label: "深色" },
  { value: "system", label: "跟随系统" },
];

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mt-[15px] first:mt-0">
      <h2 className="mb-2 ml-0.5 text-label tracking-[0.06em] text-fg3">{title}</h2>
      <div className="panel">{children}</div>
    </section>
  );
}

function Row({ title, sub, children }: { title: string; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex items-center gap-[18px] border-b border-hair px-[22px] py-3 last:border-b-0">
      <div className="min-w-0 flex-1">
        <div className="text-body">{title}</div>
        {sub && <div className="mt-[3px] text-[11.5px] leading-[1.55] text-fg3">{sub}</div>}
      </div>
      {children && <div className="flex flex-none items-center gap-2.5">{children}</div>}
    </div>
  );
}

/**
 * 一键排查（design-2.2.md §6.4）：用户在 Claude 里报错时第一反应就是来设置页翻。
 * 和概览页的连通链读同一份 useHealth() —— 两处永远说同一句话。
 * 本地能判断的实时算，服务商连通要真发请求，点「开始检查」才测。
 */
function Diagnostics() {
  const { testProviders, setPage } = useAppStore();
  const { links } = useHealth();
  const qc = useQueryClient();
  const [running, setRunning] = useState(false);
  const [ran, setRan] = useState(false);

  const run = async () => {
    setRunning(true);
    await Promise.all([
      qc.invalidateQueries({ queryKey: ["proxy-status"] }),
      qc.invalidateQueries({ queryKey: ["applied-state"] }),
      qc.invalidateQueries({ queryKey: ["pending-apply"] }),
      testProviders(),
    ]);
    setRunning(false);
    setRan(true);
  };

  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[22px] py-[13px]">
      <div className="min-w-0 flex-1">
        <div className="text-body">一键排查</div>
        <div className="mt-[3px] text-[12px] text-fg3">
          Claude 里连不上、模型不见了、突然变贵了？先点这里，三秒出结论
        </div>
        <div className="mt-[9px] flex flex-wrap gap-[7px]">
          {links.map((l) => {
            // Claude 那一环的修法在概览页（应用按钮在那儿）
            const fix =
              l.fix ??
              (l.key === "claude" && (l.tone === "attention" || l.tone === "bad")
                ? { label: "去概览页", run: () => setPage("overview") }
                : undefined);
            const cls = cn(
              "flex items-center gap-1.5 rounded-[7px] px-[9px] py-1 text-[12px] inset-ring",
              l.tone === "ok" && "text-fg2 inset-ring-hair2",
              l.tone === "idle" && "text-fg3 inset-ring-hair2",
              l.tone === "attention" && "text-accent inset-ring-accent/35",
              l.tone === "bad" && "text-danger inset-ring-danger/30",
            );
            const body = (
              <>
                <i
                  className={cn(
                    "size-[5px] flex-none rounded-full",
                    l.tone === "ok" ? "bg-ok" : l.tone === "bad" ? "bg-danger" : l.tone === "attention" ? "bg-accent" : "bg-fg3/45",
                  )}
                />
                {l.title}：{l.text}
                {l.mono && <span className="mono text-fg3">{l.mono}</span>}
              </>
            );
            return fix ? (
              <button
                key={l.key}
                onClick={fix.run}
                title={[l.hint, fix.label].filter(Boolean).join("\n")}
                className={cn(cls, "transition-colors", l.tone === "bad" ? "hover:bg-danger/5" : "hover:bg-hair")}
              >
                {body} →
              </button>
            ) : (
              <span key={l.key} className={cls} title={l.hint}>
                {body}
              </span>
            );
          })}
        </div>
      </div>
      <Button variant="ghost" onClick={() => void run()} disabled={running}>
        {running && <Loader2 className="animate-spin" />}
        {ran ? "重新检查" : "开始检查"}
      </Button>
    </div>
  );
}

/**
 * 一键使用 Winhao 的配置（design-2.2.md §6.4）：把 Claude Desktop 工作区设置里的开关调成作者日常用的。
 * 状态读的是 Claude 那边真正写着的值 —— 用户在 Claude 里自己改过，这里照实显示「有 N 项不同」。
 */
function WinhaoPresetRow() {
  const { applyState } = useAppStore();
  const [open, setOpen] = useState(false);
  const presetQ = useQuery({ queryKey: ["winhao-preset"], queryFn: winhaoPresetState, refetchOnWindowFocus: true });
  const st = presetQ.data;
  const diff = st ? st.items.filter((i) => i.supported && i.current !== i.want).length : 0;

  let sub: ReactNode;
  if (!st) sub = "读取中…";
  else if (!st.found) sub = "还没接入 Claude Desktop，先在概览页应用一次";
  else if (diff === 0)
    sub = (
      <span className="flex items-center gap-1 text-ok">
        <Check className="size-3" strokeWidth={2.6} />
        Claude 里已经是 Winhao 的配置
      </span>
    );
  else sub = `打开 Auto 模式和高级文件分析，跳过网页抓取的域名检查等 · 和现在有 ${diff} 项不同`;

  return (
    <>
      <Row title="一键使用 Winhao 的配置" sub={sub}>
        <Button
          variant="ghost"
          size="sm"
          disabled={!st?.found || applyState === "applying"}
          onClick={() => {
            void presetQ.refetch();
            setOpen(true);
          }}
        >
          {diff > 0 ? "查看并使用" : "查看"}
        </Button>
      </Row>
      <WinhaoPresetDialog open={open} onOpenChange={setOpen} state={st} />
    </>
  );
}

/** 设置页（design-2.2.md §6.4）：常用 / 高级 / 出问题时。 */
export function SettingsPage() {
  const { pref, setPref } = useTheme();
  const { changePort, draft, updateDraft, reloadConfig, focusPortNonce } = useAppStore();
  const updater = useUpdaterCtx();
  const qc = useQueryClient();

  const versionQ = useQuery({ queryKey: ["gui-version"], queryFn: guiVersion });
  const autostartQ = useQuery({ queryKey: ["autostart"], queryFn: () => isEnabled() });
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const desktopQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });

  // 端口输入（本地编辑态，blur/Enter 提交热切换）
  const portRef = useRef<HTMLInputElement>(null);
  const [portText, setPortText] = useState("");
  const [switching, setSwitching] = useState(false);
  useEffect(() => {
    if (statusQ.data) setPortText(String(statusQ.data.port));
  }, [statusQ.data]);

  // 「换一个端口」从别的页跳过来：滚到端口框并聚焦
  const handledPortNonce = useRef(0);
  useEffect(() => {
    if (focusPortNonce > handledPortNonce.current && portText) {
      handledPortNonce.current = focusPortNonce;
      portRef.current?.scrollIntoView({ block: "center" });
      // 等端口值填进输入框之后再全选，否则填值那次重渲染会把选区冲掉，用户敲的数字就接在旧端口后面了
      portRef.current?.select();
    }
  }, [focusPortNonce, portText]);

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
      await qc.invalidateQueries({ queryKey: ["available-models"] });
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

  const revealConfig = async () => {
    try {
      await revealClaudeConfig();
    } catch (e) {
      toast.error(String(e));
    }
  };

  const version = versionQ.data ?? "";
  const updateSub = updater.state.hasUpdate
    ? `当前 ${version} · 发现新版本 v${updater.state.newVersion}`
    : updater.checked
      ? `当前 ${version} · 已是最新版本`
      : `当前 ${version}`;


  const unavailable = (desktopQ.data?.unavailable ?? []).map((k) => KEY_FEATURE_NAMES[k] ?? k);
  const syncedSince = formatSince(draft?.pricing_synced_at);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title="设置" sub="常用的在上面，出问题才用的在下面。" className="pb-[18px]" />

      <div className="min-h-0 flex-1 overflow-y-auto pb-5">
        <Group title="常用">
          <WinhaoPresetRow />
          <Row title="外观">
            <Tabs value={pref} onValueChange={(v) => setPref(v as ThemePref)}>
              <TabsList>
                {THEME_TABS.map((t) => (
                  <TabsTrigger key={t.value} value={t.value}>
                    {t.label}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          </Row>
          <Row title="开机自启" sub="登录时自动启动代理">
            <Switch
              checked={autostartQ.data ?? false}
              onCheckedChange={(ck) => void toggleAutostart(ck)}
              aria-label="开机自启"
            />
          </Row>
          <Row title="软件更新" sub={<span className="mono">{updateSub}</span>}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void updater.manualCheck()}
              disabled={updater.state.isChecking}
            >
              {updater.state.isChecking && <Loader2 className="animate-spin" />}
              检查更新
            </Button>
          </Row>
        </Group>

        <Group title="高级">
          <Row
            title="代理端口"
            sub={
              statusQ.data && !statusQ.data.running ? (
                <span className="text-danger">
                  端口 {statusQ.data.port} 被别的程序占了。换一个 1024–65535 之间的数字，比如 5679，再应用一次
                </span>
              ) : (
                "改完马上生效，再应用一次 Claude 就会连到新端口"
              )
            }
          >
            {switching && <Loader2 size={12} className="animate-spin text-fg3" />}
            <Input
              ref={portRef}
              value={portText}
              onChange={(e) => setPortText(e.target.value.replace(/[^0-9]/g, ""))}
              onBlur={() => void submitPort()}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
              disabled={switching}
              inputMode="numeric"
              aria-label="代理端口"
              className="w-[92px] text-center"
            />
          </Row>
          <Row
            title="自动同步模型费率"
            sub={
              <>
                启动时从 models.dev 拉取各家官方价（最多 6 小时一次）·{" "}
                {syncedSince ? `上次同步 ${syncedSince}` : "还没同步过"}
              </>
            }
          >
            <Button variant="ghost" size="sm" onClick={() => void runSync()} disabled={syncing}>
              {syncing && <Loader2 className="animate-spin" />}
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
              aria-label="自动同步模型费率"
            />
          </Row>
        </Group>

        <Group title="出问题时">
          <Diagnostics />
          <Row
            title="Claude Desktop"
            sub={
              desktopQ.data?.version ? (
                <>
                  检测到 <span className="mono">{desktopQ.data.version}</span>
                  {unavailable.length > 0
                    ? ` · 版本过低，暂不可用：${unavailable.join("、")}`
                    : " · 全部能力可用"}
                </>
              ) : (
                "没检测到安装（写入配置时不按版本裁剪）"
              )
            }
          >
            <Button variant="ghost" size="sm" onClick={() => void revealConfig()}>
              打开配置目录
              <ExternalLink />
            </Button>
          </Row>
          <Row title="关于" sub="ModelLink by Winhao学AI · 免费软件 · 不可商业化">
            <Button variant="ghost" size="sm" onClick={() => void openUrl(GITHUB_URL)}>
              GitHub
              <ExternalLink />
            </Button>
          </Row>
        </Group>
      </div>
    </div>
  );
}
