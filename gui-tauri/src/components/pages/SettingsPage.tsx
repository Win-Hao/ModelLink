import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { GITHUB_URL } from "@/lib/constants";
import {
  appliedState,
  desktopInfo,
  guiVersion,
  proxyStatus,
  revealClaudeConfig,
  syncPricing,
  testProvider,
} from "@/lib/ipc";
import { KEY_FEATURE_NAMES, formatSince, providerDisplayName } from "@/lib/presets";
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

type Check = { key: string; state: "ok" | "bad" | "pending"; text: string; fix?: () => void };

/**
 * 一键排查（design-2.2.md §6.4）：用户在 Claude 里报错时第一反应就是来设置页翻。
 * 四项检查常驻在页面上；本地能判断的实时算，服务商连通要真发请求，点「检查」才测。
 */
function Diagnostics({ onFixPort }: { onFixPort: () => void }) {
  const { draft, applyState, verificationFor, recordVerification, setPage, gotoProvider } =
    useAppStore();
  const qc = useQueryClient();
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const appliedQ = useQuery({ queryKey: ["applied-state"], queryFn: appliedState });
  const [running, setRunning] = useState(false);
  const [ran, setRan] = useState(false);

  const providers = draft?.providers ?? [];
  const complete = (i: number) => {
    const p = providers[i];
    return !!p.target_url && !!p.api_key && p.models.some((m) => m.name);
  };

  const run = async () => {
    setRunning(true);
    await Promise.all([
      qc.invalidateQueries({ queryKey: ["proxy-status"] }),
      qc.invalidateQueries({ queryKey: ["applied-state"] }),
      ...providers.map(async (p, i) => {
        if (!complete(i)) return; // 没填全的不测，下面单独点名
        const model = p.models.find((m) => m.name)!.name;
        try {
          const r = await testProvider(p.target_url, p.api_key, model);
          recordVerification(p, { ok: r.ok, at: Date.now(), message: r.message });
        } catch (e) {
          recordVerification(p, { ok: false, at: Date.now(), message: String(e) });
        }
      }),
    ]);
    setRunning(false);
    setRan(true);
  };

  const checks: Check[] = [];
  const toOverview = () => setPage("overview");

  const st = statusQ.data;
  if (st) {
    checks.push(
      st.running
        ? { key: "port", state: "ok", text: `端口 ${st.port} 未被占用` }
        : { key: "port", state: "bad", text: `端口 ${st.port} 被占用，代理没能启动`, fix: onFixPort },
    );
  }

  const a = appliedQ.data;
  if (a && st) {
    if (!a.found) {
      checks.push({ key: "claude", state: "bad", text: "没找到 Claude Desktop 的配置" });
    } else if (a.provider !== "gateway" || a.gateway_url !== `http://127.0.0.1:${st.port}`) {
      checks.push({ key: "claude", state: "bad", text: "Claude Desktop 没接到 ModelLink", fix: toOverview });
    } else if (a.models.length === 0) {
      checks.push({ key: "claude", state: "bad", text: "Claude Desktop 里还没有模型", fix: toOverview });
    } else {
      checks.push({ key: "claude", state: "ok", text: "Claude Desktop 配置已写入" });
    }
  }

  if (providers.length === 0) {
    checks.push({ key: "keys", state: "bad", text: "还没有服务商", fix: () => setPage("providers") });
  } else if (running) {
    checks.push({ key: "keys", state: "pending", text: "正在测试服务商连接…" });
  } else {
    const incomplete = providers.findIndex((_, i) => !complete(i));
    const results = providers.map((p) => verificationFor(p));
    const failed = results.findIndex((v) => v && !v.ok);
    const failedCount = results.filter((v) => v && !v.ok).length;
    const untested = results.filter((v) => !v).length;
    const nameOf = (i: number) => providerDisplayName(providers[i].target_url, i);
    if (incomplete >= 0) {
      checks.push({
        key: "keys",
        state: "bad",
        text: `「${nameOf(incomplete)}」还没填完`,
        fix: () => gotoProvider(incomplete),
      });
    } else if (failed >= 0) {
      checks.push({
        key: "keys",
        state: "bad",
        text: failedCount === 1 ? `「${nameOf(failed)}」连不上` : `${failedCount} 家服务商连不上`,
        fix: () => gotoProvider(failed),
      });
    } else if (untested > 0) {
      checks.push({
        key: "keys",
        state: "pending",
        text: `${untested} 家服务商还没测试过连接`,
        fix: () => void run(),
      });
    } else {
      checks.push({
        key: "keys",
        state: "ok",
        text: providers.length === 1 ? "服务商密钥已连通" : `${providers.length} 家服务商密钥全部连通`,
      });
    }
  }

  // 最常见的售后原因：改了配置忘了点「应用」
  checks.push(
    applyState === "clean"
      ? { key: "apply", state: "ok", text: "配置已生效" }
      : applyState === "applying"
        ? { key: "apply", state: "pending", text: "正在应用…" }
        : applyState === "error"
          ? { key: "apply", state: "bad", text: "上次应用没成功", fix: toOverview }
          : { key: "apply", state: "bad", text: "配置已修改但尚未应用", fix: toOverview },
  );

  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[22px] py-[13px]">
      <div className="min-w-0 flex-1">
        <div className="text-body">一键排查</div>
        <div className="mt-[3px] text-[11.5px] text-fg3">
          Claude 里连不上、模型不见了、突然变贵了 —— 先点这里，三秒出结论
        </div>
        <div className="mt-[9px] flex flex-wrap gap-[7px]">
          {checks.map((c) => {
            const cls = cn(
              "flex items-center gap-1.5 rounded-[7px] px-[9px] py-1 text-[11.5px] inset-ring",
              c.state === "ok" && "text-fg2 inset-ring-hair2",
              c.state === "pending" && "text-fg3 inset-ring-hair2",
              c.state === "bad" && "text-danger inset-ring-danger/30",
            );
            const dot = (
              <i
                className={cn(
                  "size-[5px] flex-none rounded-full",
                  c.state === "ok" ? "bg-ok" : c.state === "bad" ? "bg-danger" : "bg-hair2",
                )}
              />
            );
            return c.fix ? (
              <button
                key={c.key}
                onClick={c.fix}
                className={cn(cls, "transition-colors", c.state === "bad" ? "hover:bg-danger/5" : "hover:bg-hair")}
                title={c.state === "bad" ? "去处理" : undefined}
              >
                {dot}
                {c.text} →
              </button>
            ) : (
              <span key={c.key} className={cls}>
                {dot}
                {c.text}
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

/** 设置页（design-2.2.md §6.4）：常用 / 高级 / 出问题时。 */
export function SettingsPage() {
  const { pref, setPref } = useTheme();
  const { changePort, draft, updateDraft, reloadConfig } = useAppStore();
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
          <Row title="代理端口" sub="修改后立即生效，需要重新应用到 Claude Desktop">
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
          <Diagnostics
            onFixPort={() => {
              portRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
              portRef.current?.focus();
            }}
          />
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
