import { useEffect, useRef, useState } from "react";
import { Eye, EyeOff, Loader2, X } from "lucide-react";
import { toast } from "sonner";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  probeProvider,
  testProvider,
  type ModelEntry,
  type ModelPricing,
  type ProbeResponse,
} from "@/lib/ipc";
import {
  FAMILY_TIERS,
  MAX_MODELS,
  claims1mItDoesNotHave,
  formatContext,
  THINKING_LABELS,
  getPresetModels,
  getThinkingOptions,
  providerDisplayName,
  rawSlotForModel,
  totalModelsRaw,
} from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const inputCls =
  "mono h-[33px] rounded-[9px] border-input bg-input-bg px-[11px] text-xs md:text-xs shadow-none dark:bg-input-bg";

const fieldLabelCls = "text-[11px] font-medium tracking-[.03em] text-muted-foreground";

const PRICE_FIELDS = [
  { key: "input", label: "输入" },
  { key: "output", label: "输出" },
  { key: "cache_read", label: "缓存读" },
  { key: "cache_write", label: "缓存写" },
] as const;

/**
 * 模型费率编辑区（§3.1）：四个字段全部可选，单位「每百万 token」。
 * 留空时用 models.dev 同步来的值（占位符里显示）；手填任意一格就完全接管这一行 ——
 * 同步值是美元、手填可能是人民币，逐字段混用会把两种币种加在一起。
 */
function PricingPanel({
  pricing,
  synced,
  onChange,
}: {
  pricing?: ModelPricing;
  synced?: ModelPricing;
  onChange: (fn: (p: ModelPricing) => void) => void;
}) {
  const usd = pricing?.currency === "USD";
  const unit = usd ? "$" : "¥";
  const manual = PRICE_FIELDS.some((f) => pricing?.[f.key] != null);
  // 手填了但缺输入/输出价 —— 这一行不会被写入（Claude schema 四字段必填）
  const partial = manual && (pricing?.input == null || pricing?.output == null);
  const hasSynced = PRICE_FIELDS.some((f) => synced?.[f.key] != null);

  return (
    <div className="mb-1 ml-1 mr-1 rounded-[9px] border border-dashed bg-background px-3 py-2.5">
      <div className="flex items-center justify-between">
        <span className="text-[10.5px] text-muted-foreground">
          费率 · {unit} / 百万 token · 输入与输出必填
        </span>
        {/* §五①：预设库存人民币原价，写入时按设置页的汇率换算；
            少数本来就按美元计价的中转标 USD 跳过换算 */}
        <button
          type="button"
          onClick={() => onChange((p) => (p.currency = usd ? "" : "USD"))}
          className="mono rounded-[5px] border px-1.5 py-px text-[10px] text-muted-foreground transition-colors hover:text-foreground"
        >
          {usd ? "USD（不换算）" : "CNY（按汇率换算）"}
        </button>
      </div>
      <div className="mt-1.5 grid grid-cols-4 gap-1.5">
        {PRICE_FIELDS.map((f) => (
          <label key={f.key} className="flex flex-col gap-[3px]">
            <span className="text-[10px] text-faint">{f.label}</span>
            <Input
              value={pricing?.[f.key] ?? ""}
              placeholder={synced?.[f.key] != null ? `$${synced[f.key]}` : "—"}
              onChange={(e) => {
                const raw = e.target.value.replace(/[^0-9.]/g, "");
                const n = raw === "" ? undefined : Number(raw);
                onChange((p) => {
                  p[f.key] = n !== undefined && Number.isFinite(n) ? n : undefined;
                });
              }}
              inputMode="decimal"
              className="mono h-[27px] rounded-[7px] border-input bg-card px-2 text-[11px] md:text-[11px] shadow-none dark:bg-card"
            />
          </label>
        ))}
      </div>
      <div className="mt-1.5 text-[10px] text-faint">
        {manual
          ? partial
            ? "输入与输出价都填上才会写入（Claude 的费率表四个字段都必填，缓存价留空按 0 计）"
            : "已手填，同步值不再生效（清空全部四格即恢复自动同步）"
          : hasSynced
            ? "灰字为 models.dev 同步值（美元），留空即采用"
            : "未填 → Claude 会按 Anthropic 官方价估算该槽位，也就是假账单"}
      </div>
    </div>
  );
}

/** 深度探测结果面板（§5.6）。 */
function ProbePanel({ data, onClose }: { data: ProbeResponse; onClose: () => void }) {
  const r = data.report;
  const Row = ({ label, ok, note }: { label: string; ok: boolean; note?: string }) => (
    <div className="flex items-center gap-2 py-[3px]">
      <span className={cn("flex-none text-[10px]", ok ? "text-success" : "text-destructive")}>
        {ok ? "✓" : "✗"}
      </span>
      <span className="flex-1 text-[11px] text-muted-foreground">{label}</span>
      {note && <span className="mono flex-none text-[10px] text-faint">{note}</span>}
    </div>
  );

  return (
    <div className="rounded-[9px] border bg-background px-3 py-2.5">
      <div className="flex items-baseline justify-between">
        <span className="text-[11px] font-semibold text-foreground">
          探测结果 {r.ok ? "" : "· 未连通"}
        </span>
        <button
          onClick={onClose}
          className="mono text-[10px] text-faint transition-colors hover:text-muted-foreground"
        >
          收起 · {(r.elapsed_ms / 1000).toFixed(1)}s
        </button>
      </div>

      {!r.ok ? (
        <div className="mt-1 text-[11px] text-destructive">{r.message}</div>
      ) : (
        <>
          {data.headlines.length > 0 && (
            <div className="mt-1.5 flex flex-col gap-1 border-b pb-2">
              {data.headlines.map((h, i) => (
                <span key={i} className="text-[11px] leading-[1.5] text-foreground">
                  {h}
                </span>
              ))}
            </div>
          )}
          <div className="mt-1.5">
            <Row label="基础 Messages API" ok={r.ok} />
            <Row
              label="拒绝不存在的模型名（不会静默回落）"
              ok={r.validates_model_name}
              note={r.validates_model_name ? "" : "会静默回落"}
            />
            <Row label="接受 claude-* 槽位名" ok={r.accepts_claude_slot} />
            <Row
              label="推理强度 output_config.effort"
              ok={r.effort_accepted.some(([, v]) => v)}
              note={r.effort_accepted
                .filter(([, v]) => v)
                .map(([k]) => k)
                .join("/")}
            />
            <Row
              label="原生 thinking 字段"
              ok={r.thinking_variants.some(([, v]) => v)}
              note={r.thinking_variants
                .filter(([, v]) => v)
                .map(([k]) => k)
                .join("/")}
            />
            <Row
              label="本次观察到缓存命中"
              ok={r.prompt_caching}
              note={r.prompt_caching ? "" : "合成请求未必触发"}
            />
            <Row label="1M 上下文 beta 头" ok={r.accepts_1m_beta} />
            <Row label="GET /v1/models" ok={r.models_endpoint} />
          </div>
        </>
      )}
    </div>
  );
}

/** 模型条目的进阶字段（§3.5）：层级别名 + 1M 默认。 */
function TierPanel({
  entry,
  onChange,
}: {
  entry: ModelEntry;
  onChange: (fn: (m: ModelEntry) => void) => void;
}) {
  const tier = entry.family_tier ?? "";
  return (
    <div className="mb-1 ml-1 mr-1 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-[9px] border border-dashed bg-background px-3 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-[10.5px] text-muted-foreground">层级别名</span>
        <Select
          value={tier === "" ? "none" : tier}
          onValueChange={(v) =>
            onChange((m) => {
              m.family_tier = v === "none" ? "" : v;
              if (v === "none") m.family_default = false;
            })
          }
        >
          <SelectTrigger
            size="sm"
            className="h-[26px] gap-1.5 rounded-[7px] border-input bg-card px-2 text-[11px] shadow-none dark:bg-card"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none" className="text-xs">
              不设
            </SelectItem>
            {FAMILY_TIERS.map((t) => (
              <SelectItem key={t} value={t} className="text-xs">
                {t}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* app 内该字段的 show 谓词是 !!e.anthropicFamilyTier */}
      {tier !== "" && (
        <label className="flex items-center gap-1.5">
          <Switch
            checked={!!entry.family_default}
            onCheckedChange={(ck) => onChange((m) => (m.family_default = ck))}
          />
          <span className="text-[10.5px] text-muted-foreground">该层级默认</span>
        </label>
      )}

      {/* prefer1m 的 show 谓词是 !!e.supports1m */}
      {!!entry.to_1m && (
        <label className="flex items-center gap-1.5">
          <Switch
            checked={!!entry.prefer_1m}
            onCheckedChange={(ck) => onChange((m) => (m.prefer_1m = ck))}
          />
          <span className="text-[10.5px] text-muted-foreground">默认用 1M 变体</span>
        </label>
      )}

      <span className="w-full text-[10px] text-faint">
        层级别名把 Claude 里的裸称呼（如「opus」）指到这条；opus / fable 还带拒答回退链路。留空即不参与。
      </span>
    </div>
  );
}

/** 服务商编辑器（design.md §6.2 右栏）：一次只编辑一个服务商。 */
export function ProviderEditor({ index }: { index: number }) {
  const {
    draft,
    updateDraft,
    focusKeyNonce,
    setTestedOk,
    resetTested,
    setSelectedProvider,
    applyState,
  } = useAppStore();

  const [showKey, setShowKey] = useState(false);
  const [testing, setTesting] = useState(false);
  /** 展开费率面板的模型下标（一次只开一个）。 */
  const [pricingOpen, setPricingOpen] = useState<number | null>(null);
  const [probing, setProbing] = useState(false);
  const [probe, setProbe] = useState<ProbeResponse | null>(null);
  /** 展开层级面板的模型下标。 */
  const [tierOpen, setTierOpen] = useState<number | null>(null);

  // 预设引导流：跳入本页时聚焦密钥输入框
  const keyRef = useRef<HTMLInputElement>(null);
  const handledFocusNonce = useRef(0);
  useEffect(() => {
    if (focusKeyNonce > handledFocusNonce.current) {
      handledFocusNonce.current = focusKeyNonce;
      keyRef.current?.focus();
    }
  }, [focusKeyNonce]);

  // 切换服务商时清掉编辑器瞬态
  useEffect(() => {
    setShowKey(false);
    setPricingOpen(null);
    setTierOpen(null);
    setProbe(null);
  }, [index]);

  const p = draft?.providers[index];
  if (!draft || !p) return null;

  const presetModels = getPresetModels(p.target_url);
  const thinkOpts = getThinkingOptions(p.target_url);
  const capReached = totalModelsRaw(draft) >= MAX_MODELS;
  const name = providerDisplayName(p.target_url, index);
  const busy = applyState === "applying";

  // 测试反馈走 toast（2026-07-14 用户调整，原 inline 结果 6s 淡出）
  const runTest = async () => {
    const first = p.models[0]?.name;
    if (!p.target_url || !p.api_key || !first) {
      toast.error("请填写 API 地址、密钥和至少一个模型名。");
      return;
    }
    setTesting(true);
    try {
      const r = await testProvider(p.target_url, p.api_key, first);
      if (r.ok) toast.success("连接成功 (HTTP 200)");
      else toast.error(r.message);
      setTestedOk(index, r.ok);
    } catch {
      toast.error("请求失败。");
    }
    setTesting(false);
  };

  // 深度探测（§5.6）：一次问清这家支持到什么程度
  const runProbe = async () => {
    const first = p.models[0]?.name;
    if (!p.target_url || !p.api_key || !first) {
      toast.error("请填写 API 地址、密钥和至少一个模型名。");
      return;
    }
    setProbing(true);
    setProbe(null);
    try {
      const r = await probeProvider(p.target_url, p.api_key, first);
      setProbe(r);
      setTestedOk(index, r.report.ok);
      if (!r.report.ok) toast.error(r.report.message);
    } catch (e) {
      toast.error(`探测失败：${String(e)}`);
    }
    setProbing(false);
  };

  const removeProvider = () => {
    updateDraft((c) => {
      c.providers.splice(index, 1);
    });
    resetTested();
    setSelectedProvider(Math.max(0, index - 1));
  };

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto rounded-xl border bg-card p-4">
      {/* API 地址 / 密钥（上下两行，2026-07-14 用户调整：双列太挤） */}
      <div className="flex flex-col gap-[5px]">
        <label className={fieldLabelCls}>API 地址</label>
        <Input
          value={p.target_url}
          onChange={(e) =>
            updateDraft((c) => {
              c.providers[index].target_url = e.target.value;
            })
          }
          placeholder="https://…"
          className={inputCls}
        />
      </div>
      <div className="flex flex-col gap-[5px]">
        <label className={fieldLabelCls}>API 密钥</label>
        <div className="relative">
          <Input
            ref={keyRef}
            type={showKey ? "text" : "password"}
            value={p.api_key}
            onChange={(e) =>
              updateDraft((c) => {
                c.providers[index].api_key = e.target.value;
              })
            }
            placeholder="sk-…"
            className={cn(inputCls, "w-full pr-8")}
          />
          <button
            type="button"
            onClick={() => setShowKey((v) => !v)}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-faint transition-colors hover:text-muted-foreground"
            aria-label={showKey ? "隐藏密钥" : "显示密钥"}
          >
            {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
        </div>
      </div>

      {/* 模型区标签行 + 测试连接（结果弹 toast） */}
      <div className="flex items-center justify-between">
        <label className={fieldLabelCls}>模型 · 右侧为 Claude 中显示的名称</label>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            onClick={runProbe}
            disabled={probing || testing}
            className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
            title="逐项探测：推理强度档位、模型名校验、缓存透传、1M beta"
          >
            {probing && <Loader2 size={12} className="animate-spin" />}
            深度探测
          </Button>
          <Button
            variant="outline"
            onClick={runTest}
            disabled={testing || probing}
            className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
          >
            {testing && <Loader2 size={12} className="animate-spin" />}
            测试连接
          </Button>
        </div>
      </div>

      {probe && <ProbePanel data={probe} onClose={() => setProbe(null)} />}

      {/* 模型行 */}
      {p.models.map((m, mi) => {
        const slot = rawSlotForModel(draft, index, mi);
        const dlId = `ml-models-${index}-${mi}`;
        const bad1m = claims1mItDoesNotHave(m);
        const priced = PRICE_FIELDS.some(
          (f) => m.pricing?.[f.key] != null || m.pricing_synced?.[f.key] != null,
        );
        return (
          <div key={mi} className="flex flex-col">
            <div className="flex items-center gap-[9px]">
              <Input
                value={m.name}
                onChange={(e) =>
                  updateDraft((c) => {
                    c.providers[index].models[mi].name = e.target.value;
                  })
                }
                list={presetModels.length > 0 ? dlId : undefined}
                placeholder="输入或选择模型"
                className={cn(inputCls, "min-w-0 flex-1")}
              />
              {presetModels.length > 0 && (
                <datalist id={dlId}>
                  {presetModels.map((pm) => (
                    <option key={pm} value={pm} />
                  ))}
                </datalist>
              )}
              <Switch
                checked={!!m.to_1m}
                onCheckedChange={(ck) =>
                  updateDraft((c) => {
                    c.providers[index].models[mi].to_1m = ck ? "auto" : "";
                  })
                }
              />
              {/* 已知这个模型装不下 1M 却开着 —— 引擎会照发 1M beta 头，
                  上游按自己的上限截断，用户以为有 1M 其实没有 */}
              <span
                className={cn(
                  "-ml-[3px] text-[10.5px]",
                  bad1m ? "font-semibold text-warning" : "text-faint",
                )}
                title={
                  bad1m
                    ? `上游该模型上下文只有 ${formatContext(m.context_limit)}，开着 1M 不会真的生效`
                    : m.context_limit
                      ? `上游上下文 ${formatContext(m.context_limit)}`
                      : undefined
                }
              >
                1M
              </span>
              {bad1m && (
                <span className="flex-none text-[10px] font-medium text-warning">
                  仅 {formatContext(m.context_limit)}
                </span>
              )}
              {/* 费率开关：未填时用警示色 —— 不填 = Claude 按 Anthropic 官方价估算 */}
              <button
                onClick={() => setPricingOpen(pricingOpen === mi ? null : mi)}
                className={cn(
                  "flex-none rounded-[5px] border px-1.5 py-px text-[10px] transition-colors",
                  priced
                    ? "text-muted-foreground hover:text-foreground"
                    : "border-warning/40 text-warning",
                )}
                title={priced ? "编辑费率" : "未填费率：Claude 会按 Anthropic 官方价估算"}
              >
                费率
              </button>
              <button
                onClick={() => setTierOpen(tierOpen === mi ? null : mi)}
                className="flex-none rounded-[5px] border px-1.5 py-px text-[10px] text-muted-foreground transition-colors hover:text-foreground"
                title="层级别名 / 1M 默认"
              >
                层级
              </button>
              <span className="mono max-w-[110px] flex-none truncate text-[10px] text-faint">
                {slot ? `→ ${slot}` : ""}
              </span>
              <button
                onClick={() =>
                  updateDraft((c) => {
                    c.providers[index].models.splice(mi, 1);
                  })
                }
                className="flex-none text-faint transition-colors hover:text-destructive"
                aria-label="删除模型"
              >
                <X size={13} />
              </button>
            </div>
            {tierOpen === mi && (
              <TierPanel
                entry={m}
                onChange={(fn) =>
                  updateDraft((c) => {
                    fn(c.providers[index].models[mi]);
                  })
                }
              />
            )}
            {pricingOpen === mi && (
              <PricingPanel
                pricing={m.pricing}
                synced={m.pricing_synced}
                onChange={(fn) =>
                  updateDraft((c) => {
                    const entry = c.providers[index].models[mi];
                    entry.pricing ??= {};
                    fn(entry.pricing);
                  })
                }
              />
            )}
          </div>
        );
      })}

      {/* 添加模型（满槽禁用 + Tooltip，design.md §9） */}
      {capReached ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <span tabIndex={0} className="w-full">
              <Button
                variant="ghost"
                disabled
                className="h-[30px] w-full rounded-[9px] border border-dashed text-xs font-normal text-faint"
              >
                + 添加模型
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>所有服务商的模型总数最多 {MAX_MODELS} 个</TooltipContent>
        </Tooltip>
      ) : (
        <Button
          variant="ghost"
          onClick={() =>
            updateDraft((c) => {
              // 默认不开 1M：多数国产模型上下文是 200K/256K，开了只会让选择器给出
              // 一个不会真的生效的 1M 变体（models.dev 同步后会在行内标出真实上限）
              c.providers[index].models.push({ name: "", to_1m: "" });
            })
          }
          className="h-[30px] w-full rounded-[9px] border border-dashed text-xs font-normal text-faint hover:border-primary/50 hover:bg-transparent hover:text-primary"
        >
          + 添加模型
        </Button>
      )}

      {/* 底行：默认推理强度 + 删除服务商 */}
      <div className="mt-0.5 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="min-w-0">
            <div className="text-[11px] font-medium text-muted-foreground">默认推理强度</div>
            {/* 2.1-A §3.10：语义从「强制」降级为「默认档位」 */}
            <div className="mt-px truncate text-[10.5px] text-faint">
              桌面端选择器优先；此处仅在桌面端未指定时生效
            </div>
          </div>
          <Select
            value={p.thinking_effort === "" ? "default" : p.thinking_effort}
            onValueChange={(v) =>
              updateDraft((c) => {
                c.providers[index].thinking_effort = v === "default" ? "" : v;
              })
            }
          >
            <SelectTrigger
              size="sm"
              className="h-[29px] gap-2 rounded-[8px] border-input bg-input-bg px-2.5 text-xs shadow-none dark:bg-input-bg"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {thinkOpts.map((v) => (
                <SelectItem key={v || "default"} value={v || "default"} className="text-xs">
                  {THINKING_LABELS[v]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button
              variant="ghost"
              disabled={busy}
              className="h-7 px-2 text-xs font-medium text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              删除服务商
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent className="max-w-sm">
            <AlertDialogHeader>
              <AlertDialogTitle>删除服务商「{name}」？</AlertDialogTitle>
              <AlertDialogDescription>
                将移除该服务商及其 {p.models.length} 个模型的接入配置，此操作不可撤销。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction
                onClick={removeProvider}
                className="bg-destructive text-white hover:bg-destructive/90"
              >
                删除
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </div>
  );
}
