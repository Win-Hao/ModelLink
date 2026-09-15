import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Check, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PRESET_LABELS, onOff } from "@/lib/claudePreset";
import { applyWinhaoPreset, type PresetItem, type PresetState } from "@/lib/ipc";
import { cn } from "@/lib/utils";

function ItemRow({ item, changing }: { item: PresetItem; changing: boolean }) {
  const label = PRESET_LABELS[item.key] ?? { name: item.key };
  return (
    <li className="flex items-start gap-3 py-2">
      <div className="min-w-0 flex-1">
        <div className={cn("text-[13px]", changing ? "font-medium text-fg" : "text-fg2")}>{label.name}</div>
        {changing && label.hint && <div className="mt-0.5 text-[12px] leading-[1.5] text-fg3">{label.hint}</div>}
      </div>
      <div className="flex-none pt-px text-[12.5px] whitespace-nowrap">
        {!item.supported ? (
          <span className="text-fg3">Claude 版本太旧，跳过</span>
        ) : changing ? (
          <>
            <span className="text-fg3">{onOff(item.current)}</span>
            <span className="px-1.5 text-fg3">→</span>
            <b className="font-semibold text-accent">{onOff(item.want)}</b>
          </>
        ) : (
          <span className="flex items-center gap-1 text-fg3">
            <Check className="size-3 text-ok" strokeWidth={2.6} />
            {onOff(item.want)}
          </span>
        )}
      </div>
    </li>
  );
}

/**
 * 「一键使用 Winhao 的配置」的确认框：把会改什么逐项摊开再动手 —— 这一步会重启 Claude，
 * 也会覆盖用户在 Claude 设置里自己拨过的开关，不能让人点完才知道改了什么。
 */
export function WinhaoPresetDialog({
  open,
  onOpenChange,
  state,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  state: PresetState | undefined;
}) {
  const qc = useQueryClient();
  const [applying, setApplying] = useState(false);
  const items = state?.items ?? [];
  const changes = items.filter((i) => i.supported && i.current !== i.want);
  const same = items.filter((i) => i.supported && i.current === i.want);
  const skipped = items.filter((i) => !i.supported);

  const apply = async () => {
    setApplying(true);
    try {
      await applyWinhaoPreset();
      toast.success("已换成 Winhao 的配置，Claude Desktop 正在重启…");
      onOpenChange(false);
    } catch (e) {
      toast.error(`没能写入：${String(e)}`);
    } finally {
      setApplying(false);
      await qc.invalidateQueries({ queryKey: ["winhao-preset"] });
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !applying && onOpenChange(o)}>
      <DialogContent className="gap-0 p-0 sm:max-w-[520px]">
        <DialogHeader className="px-6 pt-5 pb-3">
          <DialogTitle>使用 Winhao 的配置</DialogTitle>
          <DialogDescription>
            把 Claude Desktop 设置里「工作区」那一页的开关调成作者日常用的。以后在 Claude 的设置里还能单独改。
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[380px] overflow-y-auto border-y border-hair px-6 py-2">
          {changes.length > 0 ? (
            <section>
              <h3 className="pt-2 text-label text-fg3">会改的 {changes.length} 项</h3>
              <ul className="divide-y divide-hair">
                {changes.map((i) => (
                  <ItemRow key={i.key} item={i} changing />
                ))}
              </ul>
            </section>
          ) : (
            <p className="flex items-center gap-1.5 py-3 text-[13px] text-ok">
              <Check className="size-3.5" strokeWidth={2.6} />
              Claude 里已经是 Winhao 的配置，没有要改的
            </p>
          )}
          {same.length > 0 && (
            <section className="mt-2">
              <h3 className="pt-2 text-label text-fg3">已经一样的 {same.length} 项</h3>
              <ul className="divide-y divide-hair">
                {same.map((i) => (
                  <ItemRow key={i.key} item={i} changing={false} />
                ))}
              </ul>
            </section>
          )}
          {skipped.length > 0 && (
            <section className="mt-2 pb-1">
              <h3 className="pt-2 text-label text-fg3">装的 Claude 版本不支持、会跳过的 {skipped.length} 项</h3>
              <ul className="divide-y divide-hair">
                {skipped.map((i) => (
                  <ItemRow key={i.key} item={i} changing={false} />
                ))}
              </ul>
            </section>
          )}
        </div>

        <DialogFooter className="items-center px-6 py-4">
          {changes.length > 0 && <span className="mr-auto text-[12px] text-fg3">会重启 Claude Desktop</span>}
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={applying}>
            {changes.length > 0 ? "取消" : "关闭"}
          </Button>
          {changes.length > 0 && (
            <Button onClick={() => void apply()} disabled={applying}>
              {applying && <Loader2 className="animate-spin" />}
              使用并重启 Claude
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
