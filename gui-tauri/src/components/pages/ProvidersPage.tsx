import { useEffect } from "react";
import { Plus } from "lucide-react";

import { ApplyStatusArea } from "@/components/ApplyStatus";
import { PageHeader } from "@/components/PageHeader";
import { PresetGrid } from "@/components/PresetGrid";
import { ProviderAvatar, brandForUrl } from "@/components/ProviderAvatar";
import { ProviderEditor } from "@/components/ProviderEditor";
import { SetupSteps, type SetupStep } from "@/components/SetupSteps";
import { Button } from "@/components/ui/button";
import type { Provider } from "@/lib/ipc";
import { MAX_MODELS, flattenModels, providerDisplayName } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";
import { verificationText, type Verification } from "@/lib/verification";

/**
 * 首次应用之前，服务商页上保留那三步（design-2.2.md §6.5）：
 * 用户是跟着视频一步步走的，从引导页点进来之后不能就此没了「现在是第几步」。
 */
function firstRunSteps(p: Provider, name: string, v: Verification | undefined, testing: boolean): SetupStep[] {
  let key: SetupStep;
  if (!p.target_url) key = { title: "粘贴 API 密钥", sub: "先填 API 地址，再把密钥粘到右边", state: "current" };
  else if (!p.api_key) key = { title: "粘贴 API 密钥", sub: `从 ${name} 后台复制密钥，粘到下面「API 密钥」里`, state: "current" };
  else if (testing) key = { title: "粘贴 API 密钥", sub: "正在测试连接…", state: "current" };
  else if (v && !v.ok) key = { title: "粘贴 API 密钥", sub: `连不上：${verificationText(v.message)}`, state: "problem" };
  else if (v?.ok) key = { title: "粘贴 API 密钥", sub: "密钥能用", state: "done" };
  else key = { title: "粘贴 API 密钥", sub: "点「测试连接」确认密钥能用", state: "current" };

  return [
    { title: "挑一个服务商", sub: `已选 ${name}`, state: "done" },
    key,
    key.state === "done"
      ? { title: "应用到 Claude Desktop", sub: "点右上角的按钮，Claude 会自动重启", state: "current" }
      : { title: "应用到 Claude Desktop", sub: "Claude 会自动重启，模型选择器里就能看到你的模型", state: "upcoming" },
  ];
}

/**
 * 服务商页（design-2.2.md §6.2）：服务商 tab 条 + 全宽编辑器。
 * 典型用户只配 1 家 —— 一整列给一个条目是浪费，改 tab 后编辑区是满宽的 964px。
 */
export function ProvidersPage() {
  const {
    draft,
    selectedProvider,
    setSelectedProvider,
    addProviderFromPreset,
    setPickerOpen,
    verificationFor,
    isTesting,
  } = useAppStore();

  const count = draft?.providers.length ?? 0;

  // 删除后选中项越界时收敛
  useEffect(() => {
    if (draft && selectedProvider >= draft.providers.length && draft.providers.length > 0) {
      setSelectedProvider(draft.providers.length - 1);
    }
  }, [draft, selectedProvider, setSelectedProvider]);

  const used = draft ? flattenModels(draft).length : 0;
  const current = Math.min(selectedProvider, count - 1);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="服务商"
        sub={
          <>
            已添加{" "}
            <span className="mono">
              {used} / {MAX_MODELS}
            </span>{" "}
            个模型 · 编辑自动保存
          </>
        }
        right={count > 0 ? <ApplyStatusArea /> : undefined}
      />

      {count === 0 ? (
        <div className="min-h-0 overflow-y-auto pb-5">
          <PresetGrid onPick={addProviderFromPreset} />
        </div>
      ) : (
        <>
          {!draft!.last_applied_at && (
            <SetupSteps
              className="mb-4"
              steps={firstRunSteps(
                draft!.providers[current],
                providerDisplayName(draft!.providers[current].target_url, current),
                verificationFor(draft!.providers[current]),
                isTesting(draft!.providers[current]),
              )}
            />
          )}
          <div role="tablist" aria-label="服务商" className="mb-3 flex flex-none flex-wrap items-center gap-[5px]">
            {draft!.providers.map((p, i) => {
              const name = providerDisplayName(p.target_url, i);
              const v = verificationFor(p);
              return (
                <button
                  key={i}
                  role="tab"
                  aria-selected={i === current}
                  onClick={() => setSelectedProvider(i)}
                  className={cn(
                    "flex h-[38px] items-center gap-2 rounded-md bg-panel pr-[13px] pl-[9px] whitespace-nowrap shadow-panel transition-shadow outline-none focus-visible:ring-[3px] focus-visible:ring-ring/40",
                    i === current && "shadow-[0_1px_2px_rgba(32,28,24,.05),0_0_0_1.5px_var(--accent)]",
                  )}
                >
                  <ProviderAvatar brand={brandForUrl(p.target_url)} letter={name[0]} size={22} />
                  <span className="text-[13px] font-semibold tracking-[-0.012em]">{name}</span>
                  <span className="mono text-[11.5px] text-fg3">{p.models.length}</span>
                  {v && (
                    <i
                      className={cn("size-[5px] rounded-full", v.ok ? "bg-ok" : "bg-danger")}
                      aria-label={v.ok ? "已连通" : "连不上"}
                    />
                  )}
                </button>
              );
            })}
            <Button
              variant="dashed"
              onClick={() => setPickerOpen(true)}
              className="h-[38px] rounded-md px-[13px] text-[12.5px] font-normal"
            >
              <Plus />
              添加服务商
            </Button>
          </div>
          <ProviderEditor index={current} />
        </>
      )}
    </div>
  );
}
