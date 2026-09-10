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
import { testProvider } from "@/lib/ipc";
import {
  MAX_MODELS,
  claims1mItDoesNotHave,
  formatContext,
  THINKING_LABELS,
  getPresetModels,
  getThinkingOptions,
  providerDisplayName,
  providerNeedsEffortDefault,
  rawSlotForModel,
  totalModelsRaw,
} from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const inputCls =
  "mono h-[33px] rounded-[9px] border-input bg-input-bg px-[11px] text-xs md:text-xs shadow-none dark:bg-input-bg";

const fieldLabelCls = "text-[11px] font-medium tracking-[.03em] text-muted-foreground";

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
  }, [index]);

  const p = draft?.providers[index];
  if (!draft || !p) return null;

  const presetModels = getPresetModels(p.target_url);
  const thinkOpts = getThinkingOptions(p.target_url);
  const capReached = totalModelsRaw(draft) >= MAX_MODELS;
  const name = providerDisplayName(p.target_url, index);
  const busy = applyState === "applying";
  // 只有落在「Claude 里没有强度选择器」的槽位上的模型才够得着服务商级默认档
  const needsEffortDefault = providerNeedsEffortDefault(draft, index);
  const staleEffort = !needsEffortDefault && p.thinking_effort !== "";

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
        <Button
          variant="outline"
          onClick={runTest}
          disabled={testing}
          className="h-[29px] rounded-[9px] bg-card px-3 text-xs font-medium shadow-none dark:border-border dark:bg-card"
        >
          {testing && <Loader2 size={12} className="animate-spin" />}
          测试连接
        </Button>
      </div>

      {/* 模型行 */}
      {p.models.map((m, mi) => {
        const slot = rawSlotForModel(draft, index, mi);
        const dlId = `ml-models-${index}-${mi}`;
        const bad1m = claims1mItDoesNotHave(m);
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

      {/* 底行：默认推理强度（仅在够得着时显示）+ 删除服务商 */}
      <div className="mt-0.5 flex items-center justify-between gap-3">
        {needsEffortDefault ? (
          <div className="flex min-w-0 items-center gap-2.5">
            <div className="min-w-0">
              <div className="text-[11px] font-medium text-muted-foreground">默认推理强度</div>
              <div className="mt-px truncate text-[10.5px] text-faint">
                这些模型的槽位在 Claude 里没有强度选择器，只能在这里设
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
        ) : staleEffort ? (
          // 下拉已隐藏（模型都在有选择器的槽位上），但配置里还留着旧值 ——
          // 它只对不带 effort 的内部请求生效，看不见却还在起作用，得给个清除入口
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-[10.5px] text-faint">
              旧的服务商级推理强度「{THINKING_LABELS[p.thinking_effort]}」已基本不生效 ——
              这些模型在 Claude 里可以逐次选
            </span>
            <button
              onClick={() =>
                updateDraft((c) => {
                  c.providers[index].thinking_effort = "";
                })
              }
              className="flex-none rounded-[5px] border px-1.5 py-px text-[10px] text-muted-foreground transition-colors hover:text-foreground"
            >
              清除
            </button>
          </div>
        ) : (
          <span />
        )}
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
