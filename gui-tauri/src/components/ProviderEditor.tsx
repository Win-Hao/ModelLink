import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowUpRight, Eye, EyeOff, Loader2, MoreHorizontal, Plus, Trash2, X } from "lucide-react";
import { toast } from "sonner";

import { ModelPicker } from "@/components/ModelPicker";
import { ProviderAvatar, brandForUrl } from "@/components/ProviderAvatar";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { availableModels, desktopInfo, type ModelEntry } from "@/lib/ipc";
import {
  MAX_MODELS,
  ONE_M_CONTEXT,
  SLOT_POOL,
  THINKING_LABELS,
  flattenModels,
  formatContext,
  formatSince,
  getThinkingOptions,
  keyPagesFor,
  modelOptions,
  providerDisplayName,
  slotAbility,
  totalModelsRaw,
} from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";
import { verificationText, type Verification } from "@/lib/verification";

// 模型区列宽（design-2.2.md §6.2）：模型 262 · 上下文 96 · 1M 上下文 238 · 在 Claude 里 flex · 删除 32
const COL = {
  model: "w-[262px] flex-none",
  ctx: "w-[96px] flex-none",
  oneM: "flex w-[238px] flex-none items-center gap-2",
  slot: "flex min-w-0 flex-1 items-center gap-2",
  del: "flex w-8 flex-none justify-end",
};

/** 验证时间：今天写 HH:MM，否则写日期。 */
function verifiedAt(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

/** 面板头上常驻的连通状态 —— 不是弹一下就消失的 toast。 */
function KeyState({ verification, testing }: { verification?: Verification; testing: boolean }) {
  const base = "flex min-w-0 items-center gap-[7px] text-[12.5px] whitespace-nowrap";
  if (testing) {
    return (
      <span className={cn(base, "text-fg3")}>
        <Loader2 className="size-3 animate-spin" />
        正在测试…
      </span>
    );
  }
  if (!verification) {
    return (
      <span className={cn(base, "text-fg3")}>
        <b className="size-1.5 flex-none rounded-full bg-hair2" />
        还没测试过连接
      </span>
    );
  }
  if (verification.ok) {
    return (
      <span className={cn(base, "text-ok")}>
        <b className="size-1.5 flex-none rounded-full bg-current" />
        已连通
        <span className="mono text-fg3">· {verifiedAt(verification.at)} 验证</span>
      </span>
    );
  }
  return (
    <span className={cn(base, "text-danger")} title={verification.message}>
      <b className="size-1.5 flex-none rounded-full bg-current" />
      连不上
      <span className="truncate text-fg3">· {verificationText(verification.message)}</span>
    </span>
  );
}

/** 1M 开关旁边那句话：开不了 / 已开启 / 可以开 / 不在清单里，各说各的。 */
function OneMHint({ m, context, unlisted }: { m: ModelEntry; context: number | null; unlisted: boolean }) {
  const cls = "truncate text-[12px] whitespace-nowrap";
  if (context !== null && context < ONE_M_CONTEXT) {
    return (
      <span className={cn(cls, m.to_1m ? "text-danger" : "text-fg3")}>
        最多 {formatContext(context)}，开不了
      </span>
    );
  }
  // 手敲的名字不在服务商清单里：可能拼错了，而有的服务商对错名也照样回 200、换成默认模型 ——
  // 「测试连接」证明不了它存在，只能在这里一直标着
  if (unlisted) {
    return (
      <span className={cn(cls, "text-accent")} title="服务商的模型清单里没有这个名字。拼错了的话，有的服务商会悄悄换成默认模型回答。">
        清单里没有这个名字
      </span>
    );
  }
  if (m.to_1m) return <span className={cn(cls, "text-fg3")}>已开启</span>;
  return <span className={cn(cls, "text-fg3")}>{context === null ? "不确定能不能开" : "可以开"}</span>;
}

/** 服务商编辑器（design-2.2.md §6.2）：面板头 · 两个字段 · 模型表 · 推理强度说明。 */
export function ProviderEditor({ index }: { index: number }) {
  const {
    draft,
    updateDraft,
    focusRequest,
    setSelectedProvider,
    applyState,
    verificationFor,
    testProviders,
    isTesting,
    modelPickRequest,
    clearModelPickRequest,
  } = useAppStore();
  const infoQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });

  const [showKey, setShowKey] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // 要自动展开的那一行（刚加出来的空行）；nonce 保证同一行也能再次触发
  const [pickSignal, setPickSignal] = useState<{ row: number; nonce: number } | null>(null);

  // 预设引导流 / 「去填 API 密钥」：跳入本页时聚焦对应的输入框
  const urlRef = useRef<HTMLInputElement>(null);
  const keyRef = useRef<HTMLInputElement>(null);
  const handledFocus = useRef(0);
  useEffect(() => {
    if (focusRequest && focusRequest.provider === index && focusRequest.nonce > handledFocus.current) {
      handledFocus.current = focusRequest.nonce;
      (focusRequest.field === "url" ? urlRef : keyRef).current?.focus();
    }
  }, [focusRequest, index]);

  // 切换服务商时清掉编辑器瞬态
  useEffect(() => {
    setShowKey(false);
    setPickSignal(null);
  }, [index]);

  // 从「添加服务商」跳过来的：这家已经配过，直接展开新行的模型选择器
  // （必须排在上面那个清瞬态的 effect 后面，否则刚设上就被清掉）
  useEffect(() => {
    if (modelPickRequest && modelPickRequest.provider === index) {
      setPickSignal({ row: modelPickRequest.row, nonce: Date.now() });
      clearModelPickRequest();
    }
  }, [modelPickRequest, index, clearModelPickRequest]);

  const p = draft?.providers[index];

  // 清单优先用 models.dev 的实时数据（带上下文上限）；拉不到退回发版快照、再退回预设
  const liveModels = useQuery({
    queryKey: ["available-models", p?.target_url ?? ""],
    queryFn: () => availableModels(p!.target_url),
    enabled: !!p?.target_url,
    staleTime: 5 * 60 * 1000,
  });

  if (!draft || !p) return null;

  const name = providerDisplayName(p.target_url, index);
  const verification = verificationFor(p);
  const testing = isTesting(p);
  const busy = applyState === "applying";
  const keyPages = keyPagesFor(p.target_url);
  // 老版 Claude 不认 labelOverride：选择器里看到的是槽位名，得告诉用户
  const slotShown = !!infoQ.data?.unavailable.includes("labelOverride");
  const capReached = totalModelsRaw(draft) >= MAX_MODELS;
  const { options, source } = modelOptions(p.target_url, liveModels.data);
  const caption =
    source === "live"
      ? `${name} 的可用模型 · 来自 models.dev${
          draft.pricing_synced_at ? `，${formatSince(draft.pricing_synced_at)}同步` : ""
        }`
      : source === "none"
        ? "这家服务商没有现成的清单，在上面直接输入模型名"
        : `${name} 的常用模型 · 还没从 models.dev 同步到最新清单`;

  // 在 Claude 里的名字以真实展开结果为准（跳过没填名字的行、封顶 MAX_MODELS）
  const flat = flattenModels(draft);
  const mine = flat.filter((f) => f.providerIndex === index);
  const withoutPicker = mine.filter((f) => f.efforts.length === 0).length;
  const holders = new Map(flat.map((f) => [f.slot, f.name]));

  // 换到别的模型正在用的名字：两个互换，不用先去改另一个腾位置
  const chooseSlot = (mi: number, slot: string) =>
    updateDraft((c) => {
      const me = c.providers[index].models[mi];
      const holder = c.providers.flatMap((pv) => pv.models).find((x) => x !== me && x.name && x.slot === slot);
      if (holder) holder.slot = me.slot;
      me.slot = slot;
    });

  const runTest = async () => {
    if (!p.target_url || !p.api_key || !p.models.some((m) => m.name)) {
      toast.error("先填好 API 地址、密钥和至少一个模型，再测试连接");
      return;
    }
    await testProviders([index]);
  };

  const removeProvider = () => {
    updateDraft((c) => {
      c.providers.splice(index, 1);
    });
    setSelectedProvider(Math.max(0, index - 1));
  };

  const editModel = (mi: number, fn: (m: ModelEntry) => void) =>
    updateDraft((c) => {
      fn(c.providers[index].models[mi]);
    });

  return (
    <div className="panel mb-5 flex min-h-0 flex-1 flex-col">
      {/* 面板头：有主语，状态和操作才有落点 */}
      <div className="flex h-14 flex-none items-center gap-3 border-b border-hair px-[22px]">
        <ProviderAvatar brand={brandForUrl(p.target_url)} letter={name[0]} size={28} />
        <span className="flex-none text-heading">{name}</span>
        <span className="h-4 w-px flex-none bg-hair2" />
        <KeyState verification={verification} testing={testing} />
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={() => void runTest()} disabled={testing}>
          {verification ? "重新测试" : "测试连接"}
        </Button>
        {/* 破坏性操作收进菜单，不和编辑控件混在一起 */}
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="size-8 rounded-ctl text-fg3" aria-label="更多操作">
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem variant="danger" disabled={busy} onSelect={() => setConfirmDelete(true)}>
              <Trash2 />
              删除服务商
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {/* 字段区：两栏等宽，同高同形同基线 */}
      <div className="grid flex-none grid-cols-2 gap-5 border-b border-hair px-[22px] pt-[18px] pb-5">
        <label className="flex min-w-0 flex-col gap-[7px]">
          <span className="flex h-4 items-center text-label text-fg3">API 地址</span>
          <Input
            ref={urlRef}
            value={p.target_url}
            onChange={(e) =>
              updateDraft((c) => {
                c.providers[index].target_url = e.target.value;
              })
            }
            placeholder="https://…"
            spellCheck={false}
            className="h-9"
          />
        </label>
        <div className="flex min-w-0 flex-col gap-[7px]">
          {/* 两栏标签行同高：右边多了拿密钥的链接，也不能把输入框往下挤（同高同形同基线） */}
          <span className="flex h-4 items-center">
            <label htmlFor={`api-key-${index}`} className="text-label text-fg3">
              API 密钥
            </label>
            {keyPages.length > 0 && (
              <span className="ml-auto flex items-center gap-2 text-[12px] text-fg3">
                {keyPages.length > 1 && <span>去后台拿密钥：</span>}
                {keyPages.map((k) => (
                  <button
                    key={k.url}
                    type="button"
                    onClick={() => void openUrl(k.url)}
                    title={k.url}
                    className="flex items-center gap-0.5 rounded-[4px] transition-colors outline-none hover:text-fg focus-visible:ring-[3px] focus-visible:ring-ring/40"
                  >
                    {k.plan ?? `去 ${name} 后台拿密钥`}
                    <ArrowUpRight className="size-3" />
                  </button>
                ))}
              </span>
            )}
          </span>
          <span className="relative">
            <Input
              ref={keyRef}
              id={`api-key-${index}`}
              type={showKey ? "text" : "password"}
              value={p.api_key}
              onChange={(e) =>
                updateDraft((c) => {
                  c.providers[index].api_key = e.target.value;
                })
              }
              placeholder="sk-…"
              spellCheck={false}
              className="h-9 pr-9"
            />
            <button
              type="button"
              onClick={() => setShowKey((v) => !v)}
              className="absolute top-1/2 right-2.5 -translate-y-1/2 text-fg3 transition-colors hover:text-fg"
              aria-label={showKey ? "隐藏密钥" : "显示密钥"}
            >
              {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </span>
        </div>
      </div>

      {/* 模型区 */}
      <div className="flex min-h-0 flex-1 flex-col px-[22px]">
        <div className="flex h-11 flex-none items-center text-label text-fg3">
          模型 · <span className="mono ml-1">{p.models.length}</span> 个
        </div>
        <div className="flex h-[26px] flex-none items-center border-b border-hair text-label text-fg3">
          <span className={COL.model}>模型</span>
          <span className={COL.ctx}>上下文</span>
          <span className={COL.oneM}>1M 上下文</span>
          <span className={COL.slot}>在 Claude 里</span>
          <span className={COL.del} />
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {p.models.map((m, mi) => {
            const slot = flat.find((f) => f.providerIndex === index && f.modelIndex === mi);
            // 上下文：同步来的上限优先；刚挑的还没落盘，先用清单里的
            const context = m.context_limit ?? options.find((o) => o.id === m.name)?.context ?? null;
            // 装不下 1M：在源头拦住。已经开着的（老配置）允许关，关上之后就不能再开
            const cannotHold = context !== null && context < ONE_M_CONTEXT;
            // 有清单时，手敲进来、清单里又没有的名字一直标着（清单来源见 modelOptions）
            const unlisted = !!m.name && options.length > 0 && !options.some((o) => o.id === m.name);
            return (
              <div key={mi} className="flex h-[50px] items-center border-b border-hair last:border-b-0">
                <span className={COL.model}>
                  <ModelPicker
                    label={m.name ? `模型：${m.name}` : `第 ${mi + 1} 行模型，还没选`}
                    value={m.name}
                    options={options}
                    caption={caption}
                    showFooter={source !== "none"}
                    openSignal={pickSignal?.row === mi ? pickSignal.nonce : 0}
                    onPick={(id, ctx) =>
                      editModel(mi, (e) => {
                        if (e.name === id) return;
                        e.name = id;
                        // 上下文和费率跟着模型名走：换了模型，旧值作废（后端保存时按新名字补）
                        e.context_limit = ctx ?? undefined;
                        e.pricing_synced = undefined;
                        if (ctx !== null && ctx < ONE_M_CONTEXT) e.to_1m = "";
                      })
                    }
                  />
                </span>
                <span className={cn(COL.ctx, "mono text-[12.5px]", context ? "text-fg2" : "text-fg3")}>
                  {context ? formatContext(context) : "—"}
                </span>
                <span className={COL.oneM}>
                  <Switch
                    checked={!!m.to_1m}
                    disabled={cannotHold && !m.to_1m}
                    onCheckedChange={(ck) =>
                      editModel(mi, (e) => {
                        e.to_1m = ck ? "auto" : "";
                      })
                    }
                    aria-label={`${m.name || `第 ${mi + 1} 行模型`} 开启 1M 上下文`}
                  />
                  <OneMHint m={m} context={context} unlisted={unlisted} />
                </span>
                <span className={cn(COL.slot, "text-[12.5px] text-fg2")}>
                  {slot ? (
                    <>
                      {/* 能用什么由 Claude 按名字决定：Auto 模式、思考档位。下拉里直接写后果，名字放小字 */}
                      <Select value={slot.slot} onValueChange={(v) => chooseSlot(mi, v)}>
                        <SelectTrigger
                          size="sm"
                          className="max-w-full min-w-0"
                          title={`Claude 内部用的名字：${slot.slot}`}
                          aria-label={`${m.name} 在 Claude 里用的名字`}
                        >
                          <SelectValue>
                            <span className={cn("truncate", slot.efforts.length === 0 && "text-fg3")}>
                              {slotAbility(slot.slot)}
                            </span>
                          </SelectValue>
                        </SelectTrigger>
                        <SelectContent position="popper" align="start" className="w-[320px]">
                          {[
                            ...SLOT_POOL.map((s) => s.id),
                            // 溢出层的名字彼此没区别，只列它自己正在用的那个
                            ...(SLOT_POOL.some((s) => s.id === slot.slot) ? [] : [slot.slot]),
                          ].map((s) => {
                            const holder = s !== slot.slot ? holders.get(s) : undefined;
                            return (
                              <SelectItem key={s} value={s} className="h-auto py-[5px]">
                                <span className="flex min-w-0 flex-col items-start gap-px">
                                  <span>{slotAbility(s)}</span>
                                  <span className="text-[11.5px] text-fg3">
                                    <span className="mono">{s}</span>
                                    {holder && ` · 和「${holder}」互换`}
                                  </span>
                                </span>
                              </SelectItem>
                            );
                          })}
                          <SelectSeparator />
                          <p className="px-[9px] py-1.5 text-[11.5px] leading-[1.5] text-balance text-fg3">
                            应用之后，Claude 里正选着这个名字的对话会换成新的模型。
                          </p>
                        </SelectContent>
                      </Select>
                      {slotShown && (
                        <span className="truncate text-fg3">
                          显示为 <span className="mono text-fg2">{slot.slot}</span>
                        </span>
                      )}
                    </>
                  ) : m.name ? (
                    // 超出 20 个的模型不会写进 Claude —— 说出来，别让它静默消失
                    <span className="truncate text-[12px] text-danger">超出 {MAX_MODELS} 个上限，不会出现在 Claude 里</span>
                  ) : (
                    <span className="truncate text-[12px] text-fg3">选好模型后出现在 Claude 里</span>
                  )}
                </span>
                <span className={COL.del}>
                  <button
                    type="button"
                    onClick={() =>
                      updateDraft((c) => {
                        c.providers[index].models.splice(mi, 1);
                      })
                    }
                    className="flex size-[26px] items-center justify-center rounded-[7px] text-fg3 transition-colors hover:bg-hair hover:text-danger"
                    aria-label={`删除模型 ${m.name}`}
                  >
                    <X size={12} strokeWidth={2.2} />
                  </button>
                </span>
              </div>
            );
          })}

          {capReached ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span tabIndex={0} className="mt-2.5 block">
                  <Button variant="dashed" disabled className="h-[34px] w-full rounded-ctl text-[12.5px]">
                    <Plus />
                    添加模型
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent>所有服务商的模型加起来最多 {MAX_MODELS} 个</TooltipContent>
            </Tooltip>
          ) : (
            <Button
              variant="dashed"
              onClick={() => {
                // 默认不开 1M：多数国产模型上下文是 200K/256K
                setPickSignal({ row: p.models.length, nonce: Date.now() });
                updateDraft((c) => {
                  c.providers[index].models.push({ name: "", to_1m: "" });
                });
              }}
              className="mt-2.5 h-[34px] w-full rounded-ctl text-[12.5px]"
            >
              <Plus />
              添加模型
            </Button>
          )}
        </div>
      </div>

      {/* 面板脚：只在有事可做时出现 —— 全都能在 Claude 里调思考深度时，这里没有要决定的东西 */}
      {mine.length > 0 && (withoutPicker > 0 || p.thinking_effort !== "") && (
        <div className="flex min-h-[46px] flex-none items-center gap-3 border-t border-hair px-[22px] py-2">
          {withoutPicker === 0 ? (
            // 下拉已经没用了，但配置里还留着旧值：它对不带强度的内部请求仍然生效，得给个清除入口
            <>
                <span className="text-[12px] leading-[1.55] text-fg3">
                  这里还留着以前设的默认思考深度「{THINKING_LABELS[p.thinking_effort] ?? p.thinking_effort}」。这家的模型在
                  Claude 里都能自己选，它基本用不上了。
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-auto"
                  onClick={() =>
                    updateDraft((c) => {
                      c.providers[index].thinking_effort = "";
                    })
                  }
                >
                  清除
                </Button>
            </>
          ) : (
            <>
              <span className="text-[12px] leading-[1.55] text-fg3">
                {withoutPicker === mine.length ? "这家的模型" : `其中 ${withoutPicker} 个模型`}在 Claude
                里不能调思考深度，按这里的默认档来。
              </span>
              <Select
                value={p.thinking_effort === "" ? "default" : p.thinking_effort}
                onValueChange={(v) =>
                  updateDraft((c) => {
                    c.providers[index].thinking_effort = v === "default" ? "" : v;
                  })
                }
              >
                <SelectTrigger size="sm" className="ml-auto flex-none">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {getThinkingOptions(p.target_url).map((v) => (
                    <SelectItem key={v || "default"} value={v || "default"}>
                      {THINKING_LABELS[v]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </>
          )}
        </div>
      )}

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除服务商「{name}」？</AlertDialogTitle>
            <AlertDialogDescription>
              会移除这家服务商和它的 {p.models.length} 个模型。应用到 Claude Desktop 之后，这些模型会从 Claude
              里消失。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={removeProvider}>
              删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
