import { PresetGrid } from "@/components/PresetGrid";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { matchPreset } from "@/lib/presets";
import { useAppStore } from "@/lib/store";

/** 「添加服务商」预设网格弹窗：顶栏 [+] 与服务商页 tab 条末尾共用。 */
export function ProviderPickerDialog() {
  const { draft, pickerOpen, setPickerOpen, addProviderFromPreset } = useAppStore();
  const configured = new Set(
    (draft?.providers ?? []).flatMap((p) => matchPreset(p.target_url)?.id ?? []),
  );

  return (
    <Dialog open={pickerOpen} onOpenChange={setPickerOpen}>
      <DialogContent className="gap-4 bg-bg p-5 sm:max-w-[640px]">
        <DialogHeader>
          <DialogTitle>选择一个服务商</DialogTitle>
          {configured.size > 0 && (
            <DialogDescription className="text-[12px] text-fg3">
              标着「已添加」的点进去是给它加模型，不会再建一个重复的
            </DialogDescription>
          )}
        </DialogHeader>
        <PresetGrid
          configured={configured}
          onPick={(p) => {
            setPickerOpen(false);
            addProviderFromPreset(p);
          }}
        />
      </DialogContent>
    </Dialog>
  );
}
