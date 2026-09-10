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
import {
  DEFAULT_HEARTBEAT_SECS,
  DEFAULT_USD_RATE,
  KEY_FEATURE_NAMES,
  ORG_INSTRUCTIONS_MAX,
  formatAppliedAt,
} from "@/lib/presets";
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

  // 汇率输入（本地编辑态，blur/Enter 提交；非法值回滚到当前值）
  const [rateText, setRateText] = useState("");
  useEffect(() => {
    if (draft) setRateText(String(draft.usd_rate ?? DEFAULT_USD_RATE));
  }, [draft?.usd_rate]);

  const submitRate = () => {
    const cur = draft?.usd_rate ?? DEFAULT_USD_RATE;
    const n = Number(rateText);
    if (!rateText || !Number.isFinite(n) || n <= 0) {
      toast.error("汇率需大于 0");
      setRateText(String(cur));
      return;
    }
    if (n !== cur) updateDraft((c) => (c.usd_rate = n));
  };

  const orgLen = (draft?.org_instructions ?? "").length;

  // §3.9 代理地址校验，与后端 egress_proxy_url_valid 同一套规则
  const badProxy = (v: string) => {
    const u = v.trim();
    if (!u) return false;
    if (!/^https?:\/\//.test(u)) return true;
    return (u.split("://")[1] ?? "").split("/")[0].includes("@");
  };
  const proxyError =
    badProxy(draft?.egress_proxy_url ?? "") || badProxy(draft?.egress_proxy_pac_url ?? "")
      ? "地址无效：需以 http:// 或 https:// 开头，且不能内嵌账号密码"
      : null;
  const pacSet = !!(draft?.egress_proxy_pac_url ?? "").trim();

  const desktopQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });
  const unavailable = (desktopQ.data?.unavailable ?? []).map(
    (k) => KEY_FEATURE_NAMES[k] ?? k,
  );

  // 心跳间隔（本地编辑态，blur/Enter 提交）
  const [hbText, setHbText] = useState("");
  useEffect(() => {
    if (draft) setHbText(String(draft.heartbeat_secs ?? DEFAULT_HEARTBEAT_SECS));
  }, [draft?.heartbeat_secs]);

  const submitHeartbeat = () => {
    const cur = draft?.heartbeat_secs ?? DEFAULT_HEARTBEAT_SECS;
    const n = Number(hbText);
    // 上界 120：引擎约 5 分钟判死线，间隔再大就起不到保活作用了
    if (hbText === "" || !Number.isInteger(n) || n < 0 || n > 120) {
      toast.error("心跳间隔需在 0–120 秒之间（0 = 关闭）");
      setHbText(String(cur));
      return;
    }
    if (n !== cur) updateDraft((c) => (c.heartbeat_secs = n));
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

          {/* 隐藏调用优化（§5.5.4） */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">标题生成省思考</div>
              <div className="mt-px text-[11px] text-faint">
                每开一个新会话，Claude 会发一次「起标题」请求，实测花掉 88 个思考 token。
                开启后把它降到最低档，标题质量没有可见变化
              </div>
            </div>
            <Switch
              checked={draft?.optimize_title_gen ?? true}
              disabled={!draft}
              onCheckedChange={(ck) =>
                updateDraft((c) => {
                  c.optimize_title_gen = ck;
                })
              }
            />
          </div>

          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">健康检查本地应答</div>
              <div className="mt-px text-[11px] text-faint">
                Claude 会定期发一个 1 token 的探活请求。开启后由 ModelLink 直接回复，不打上游
                —— 只省 8 个 token，但上游真挂了也会显示正常，所以默认关
              </div>
            </div>
            <Switch
              checked={draft?.short_circuit_health_check ?? false}
              disabled={!draft}
              onCheckedChange={(ck) =>
                updateDraft((c) => {
                  c.short_circuit_health_check = ck;
                })
              }
            />
          </div>

          {/* 网络代理（2.1-E §3.9）：给挂梯子 / 公司代理的用户 */}
          <div className="flex flex-col gap-2 border-t px-4 py-3">
            <div>
              <div className="text-[13px] font-medium">Claude 出网代理</div>
              <div className="mt-px text-[11px] text-faint">
                仅 http:// 或 https://，不支持 SOCKS，也不能内嵌账号密码。127.0.0.1
                自动绕过，不影响 ModelLink 本地网关。代理不通会直接失败而不是回落直连；改完需重启
                Claude 生效。
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Input
                value={draft?.egress_proxy_url ?? ""}
                onChange={(e) =>
                  updateDraft((c) => {
                    c.egress_proxy_url = e.target.value;
                  })
                }
                disabled={!draft || pacSet}
                placeholder="http://proxy.example.com:8080"
                className="mono h-[30px] rounded-[9px] border-input bg-input-bg px-[11px] text-xs md:text-xs shadow-none dark:bg-input-bg"
              />
              <Input
                value={draft?.egress_proxy_pac_url ?? ""}
                onChange={(e) =>
                  updateDraft((c) => {
                    c.egress_proxy_pac_url = e.target.value;
                  })
                }
                disabled={!draft}
                placeholder="PAC 地址（可选，填了就压过上面那条）"
                className="mono h-[30px] rounded-[9px] border-input bg-input-bg px-[11px] text-xs md:text-xs shadow-none dark:bg-input-bg"
              />
              {proxyError && <span className="text-[10.5px] text-destructive">{proxyError}</span>}
              {pacSet && !proxyError && (
                <span className="text-[10.5px] text-faint">
                  已填 PAC，上面的普通代理会被 Claude 忽略
                </span>
              )}
            </div>
          </div>

          {/* 组织级指令（2.1-D §3.7） */}
          <div className="flex flex-col gap-2 border-t px-4 py-3">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="text-[13px] font-medium">组织级指令</div>
                <div className="mt-px text-[11px] text-faint">
                  追加到 Chat / Cowork / Code 的系统提示词（含它们派生的子 agent）。Claude
                  会告诉模型「这来自管理员，优先于个人偏好」—— 是引导，不是强制约束。
                </div>
              </div>
              <span className="mono flex-none pt-0.5 text-[10.5px] text-faint">
                {orgLen} / {ORG_INSTRUCTIONS_MAX}
              </span>
            </div>
            <textarea
              value={draft?.org_instructions ?? ""}
              onChange={(e) =>
                updateDraft((c) => {
                  c.org_instructions = e.target.value.slice(0, ORG_INSTRUCTIONS_MAX);
                })
              }
              disabled={!draft}
              rows={3}
              placeholder="例：统一用简体中文回答；代码一律带类型注解。"
              className="w-full resize-y rounded-[9px] border border-input bg-input-bg px-[11px] py-2 text-xs leading-[1.6] outline-none placeholder:text-faint focus-visible:border-ring dark:bg-input-bg"
            />
            <div className="flex items-center justify-between gap-4">
              <div className="text-[11px] text-faint">
                在指令前追加槽位映射说明
                <br />
                Chat 模式跑的是 Claude Code 引擎，系统提示词里有一句第二人称的「You are a Claude
                agent」，实测会让部分国产模型自称 Claude；这段映射把真实身份摊给模型。
              </div>
              <Switch
                checked={draft?.org_identity_note ?? true}
                disabled={!draft}
                onCheckedChange={(ck) =>
                  updateDraft((c) => {
                    c.org_identity_note = ck;
                  })
                }
              />
            </div>
          </div>

          {/* SSE 心跳（2.1-C §3.2）：治长生成断流 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">流式心跳间隔</div>
              <div className="mt-px text-[11px] text-faint">
                上游思考期间每隔这么久往 Claude 发一次保活，避免长生成被判断流 · 0 = 关闭
              </div>
            </div>
            <div className="flex flex-none items-center gap-1.5">
              <Input
                value={hbText}
                onChange={(e) => setHbText(e.target.value.replace(/[^0-9]/g, ""))}
                onBlur={submitHeartbeat}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                }}
                disabled={!draft}
                inputMode="numeric"
                className="mono h-[29px] w-[64px] rounded-[9px] border-input bg-input-bg px-2.5 text-center text-xs md:text-xs shadow-none dark:bg-input-bg"
              />
              <span className="text-[11px] text-faint">秒</span>
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

          {/* 人民币兑美元汇率（2.1-B §五①）：费率表单位写死 USD，换算必须可见可改 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">人民币兑美元汇率</div>
              <div className="mt-px text-[11px] text-faint">
                模型费率按人民币填写，写入 Claude Desktop 时除以该汇率换算成美元
              </div>
            </div>
            <Input
              value={rateText}
              onChange={(e) => setRateText(e.target.value.replace(/[^0-9.]/g, ""))}
              onBlur={submitRate}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
              disabled={!draft}
              inputMode="decimal"
              className="mono h-[29px] w-[88px] rounded-[9px] border-input bg-input-bg px-2.5 text-center text-xs md:text-xs shadow-none dark:bg-input-bg"
            />
          </div>

          {/* 兼容模式（2.1-A §3.3）：给依赖旧版静默回落行为的用户留的台阶 */}
          <div className="flex items-center justify-between border-t px-4 py-3">
            <div className="pr-4">
              <div className="text-[13px] font-medium">未映射槽位回落（兼容模式）</div>
              <div className="mt-px text-[11px] text-faint">
                关闭时，Claude 请求了没配置的模型槽位会直接报错；开启则沿用旧版行为，
                悄悄改用第一个模型
              </div>
            </div>
            <Switch
              checked={draft?.compat_fallback ?? false}
              disabled={!draft}
              onCheckedChange={(ck) =>
                updateDraft((c) => {
                  c.compat_fallback = ck;
                })
              }
            />
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
