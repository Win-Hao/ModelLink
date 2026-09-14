import { useQuery } from "@tanstack/react-query";
import { CircleCheck, Copy } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { desktopInfo, proxyStatus } from "@/lib/ipc";
import { isWindows } from "@/lib/platform";
import { flattenModels } from "@/lib/presets";
import { useAppStore } from "@/lib/store";

function CopyValue({ value }: { value: string }) {
  return (
    <button
      type="button"
      onClick={() =>
        void navigator.clipboard.writeText(value).then(
          () => toast.success(`已复制 ${value}`),
          () => toast.error("复制失败，请手动输入"),
        )
      }
      className="mono inline-flex h-6 items-center gap-1.5 rounded-[6px] bg-sunken px-2 align-middle text-[12px] text-fg inset-ring inset-ring-hair2 transition-colors hover:bg-hair"
      aria-label={`复制 ${value}`}
    >
      {value}
      <Copy className="size-3 text-fg3" />
    </button>
  );
}

/**
 * 首次应用成功后的交接（design-2.2.md §6.5）：ModelLink 的活干完了，价值在 Claude 里兑现 ——
 * 告诉用户下一步去哪、选哪个。Windows 还差一步 ModelLink 写不了的手动设置，照 README 原样列出。
 */
export function HandoffCard() {
  const { handoff, dismissHandoff, draft } = useAppStore();
  const infoQ = useQuery({ queryKey: ["desktop-info"], queryFn: desktopInfo });
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  if (!handoff || !draft) return null;

  const first = flattenModels(draft).find((m) => m.name === handoff.model) ?? flattenModels(draft)[0];
  // 老版 Claude 不认 labelOverride，选择器里看到的是槽位名
  const shownAs = infoQ.data?.unavailable.includes("labelOverride") ? first?.slot : first?.name;
  const port = statusQ.data?.port ?? draft.port ?? 5678;
  const windows = isWindows();

  return (
    <section aria-label="接入完成" className="panel mb-2.5 flex flex-none items-start gap-3 px-[22px] py-4">
      <CircleCheck className="mt-px size-[18px] flex-none text-ok" />
      <div className="min-w-0 flex-1">
        <h2 className="text-heading">{windows ? "配置已写入 Claude Desktop" : "接入完成，Claude Desktop 正在重启"}</h2>
        {shownAs && (
          <p className="mt-1 text-[13px] leading-[1.6] text-fg2">
            {windows ? "做完下面这一步之后，" : "打开 Claude 后，"}在模型选择器里选 <b className="mono font-semibold text-fg">{shownAs}</b>{" "}
            就能开始聊。
          </p>
        )}
        {windows && (
          <>
            <p className="mt-3 text-[13px] font-semibold">Windows 还差一步（只需要做一次）</p>
            <ol className="mt-1.5 list-decimal space-y-1.5 pl-5 text-[13px] leading-[1.7] text-fg2 marker:text-fg3">
              <li>
                打开 Claude Desktop，点左上角的菜单按钮 → <b className="font-medium text-fg">Developer</b> →{" "}
                <b className="font-medium text-fg">Configure third-party inference</b>
              </li>
              <li>
                点左下角切到 <b className="font-medium text-fg">Form view</b>
              </li>
              <li>
                <b className="font-medium text-fg">Gateway URL</b> 填 <CopyValue value={`http://127.0.0.1:${port}`} />
                ，<b className="font-medium text-fg">API Key</b> 填 <CopyValue value="proxy" />
              </li>
              <li>
                点 <b className="font-medium text-fg">Apply locally</b>
              </li>
            </ol>
          </>
        )}
      </div>
      <Button variant="ghost" size="sm" onClick={dismissHandoff} className="flex-none">
        知道了
      </Button>
    </section>
  );
}
