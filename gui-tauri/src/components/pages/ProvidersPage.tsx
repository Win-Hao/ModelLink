import { useEffect, useState } from "react";

import { ApplyPill } from "@/components/ApplyStatus";
import { PresetGrid } from "@/components/PresetGrid";
import { ProviderAvatar, logoForUrl } from "@/components/ProviderAvatar";
import { ProviderEditor } from "@/components/ProviderEditor";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { MAX_MODELS, flattenModels, providerDisplayName } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

/** 服务商页（design.md §6.2）：左列清单 + 右侧编辑器；空状态铺预设网格。 */
export function ProvidersPage() {
  const {
    draft,
    selectedProvider,
    setSelectedProvider,
    addProviderFromPreset,
    testedOk,
  } = useAppStore();
  const [pickerOpen, setPickerOpen] = useState(false);

  const count = draft?.providers.length ?? 0;

  // 删除后选中项越界时收敛
  useEffect(() => {
    if (draft && selectedProvider >= draft.providers.length && draft.providers.length > 0) {
      setSelectedProvider(draft.providers.length - 1);
    }
  }, [draft, selectedProvider, setSelectedProvider]);

  const used = draft ? flattenModels(draft).length : 0;

  return (
    <>
      <header className="flex items-end justify-between gap-3.5 px-6 pb-3.5 pt-[46px]">
        <div>
          <h1 className="text-[19px] font-[650] leading-[1.25] tracking-[-0.01em]">服务商</h1>
          <p className="mt-[3px] text-xs text-muted-foreground">
            <span className="mono">
              {used} / {MAX_MODELS}
            </span>{" "}
            个模型槽位已使用
          </p>
        </div>
        <ApplyPill />
      </header>

      <div className="flex flex-1 flex-col overflow-hidden px-6 pb-6 pt-0.5">
        {count === 0 ? (
          // 空状态：双栏隐藏，整个区域铺预设网格
          <div className="flex flex-col gap-3.5 overflow-y-auto">
            <PresetGrid onPick={addProviderFromPreset} />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 gap-3.5">
            {/* 左列：服务商清单 */}
            <div className="flex w-[196px] flex-none flex-col gap-2 overflow-y-auto">
              {draft!.providers.map((p, i) => {
                const name = providerDisplayName(p.target_url, i);
                return (
                  <button
                    key={i}
                    onClick={() => setSelectedProvider(i)}
                    className={cn(
                      "flex items-center gap-[9px] rounded-[10px] border bg-card px-3 py-2.5 text-left transition-colors",
                      i === selectedProvider
                        ? "border-[rgba(217,119,87,.5)]"
                        : "hover:border-primary/30",
                    )}
                  >
                    <ProviderAvatar logo={logoForUrl(p.target_url)} letter={name[0]} />
                    <span className="min-w-0">
                      <span className="block truncate text-[12.5px] font-semibold text-foreground">
                        {name}
                      </span>
                      <span className="mt-px block text-[10.5px] text-faint">
                        {p.models.length} 个模型
                        {testedOk[i] ? " · 已连通" : ""}
                      </span>
                    </span>
                  </button>
                );
              })}
              <button
                onClick={() => setPickerOpen(true)}
                className="flex h-8 w-full flex-none items-center justify-center rounded-[9px] border border-dashed text-xs text-faint transition-colors hover:border-primary/50 hover:text-primary"
              >
                + 添加服务商
              </button>
            </div>

            {/* 右侧：编辑器 */}
            <ProviderEditor index={Math.min(selectedProvider, count - 1)} />
          </div>
        )}
      </div>

      {/* 「+ 添加服务商」→ 预设网格 Dialog 形态（design.md §7） */}
      <Dialog open={pickerOpen} onOpenChange={setPickerOpen}>
        <DialogContent className="max-w-[600px] gap-4 rounded-xl bg-background p-5">
          <DialogHeader>
            <DialogTitle className="text-[15px]">选择一个服务商</DialogTitle>
          </DialogHeader>
          <PresetGrid
            onPick={(p) => {
              setPickerOpen(false);
              addProviderFromPreset(p);
            }}
          />
        </DialogContent>
      </Dialog>
    </>
  );
}
