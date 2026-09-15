import { Fragment } from "react";
import { MoreHorizontal } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { revealClaudeConfig } from "@/lib/ipc";
import { worse, type HealthLink, type Tone } from "@/lib/health";
import { isWindows } from "@/lib/platform";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const DOT: Record<Tone, string> = {
  ok: "bg-ok",
  idle: "bg-fg3/45",
  attention: "bg-accent",
  bad: "bg-danger",
};
const TEXT: Record<Tone, string> = {
  ok: "text-fg2",
  idle: "text-fg3",
  attention: "text-accent",
  bad: "text-danger",
};

/** 两环之间的连线：两端都好是实线；有一端要动手 / 坏了，这段就断成虚线。 */
function Connector({ tone }: { tone: Tone }) {
  const color = tone === "bad" ? "text-danger" : tone === "attention" ? "text-accent" : "text-hair2 dark:text-white/20";
  return (
    <span aria-hidden className={cn("flex min-w-8 flex-1 items-center px-2", color)}>
      <span
        className={cn(
          "h-0 flex-1 border-t border-current",
          (tone === "bad" || tone === "attention") && "border-dashed",
        )}
      />
      <svg width="6" height="9" viewBox="0 0 6 9" fill="none" className="-ml-px flex-none">
        <path d="M.75.75 5 4.5.75 8.25" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </span>
  );
}

function Node({ link }: { link: HealthLink }) {
  return (
    <div className="flex min-w-0 flex-none items-center gap-3" title={link.hint}>
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <i className={cn("size-[7px] flex-none rounded-full", DOT[link.tone])} />
          <span className="text-[13px] font-semibold tracking-[-0.01em] whitespace-nowrap">{link.title}</span>
        </div>
        <div className={cn("mt-[3px] truncate pl-[15px] text-[12.5px] leading-[1.45]", TEXT[link.tone])}>
          {link.text}
          {link.mono && (
            <>
              <span className="text-fg3"> · </span>
              <span className="mono text-fg3">{link.mono}</span>
            </>
          )}
        </div>
      </div>
      {link.fix && (
        <Button variant="ghost" size="sm" onClick={link.fix.run} className="flex-none">
          {link.fix.label}
        </Button>
      )}
    </div>
  );
}

/**
 * 连通链（design-2.2.md §6.1）：Claude Desktop → ModelLink → 服务商。
 * 请求就是按这个方向走的；哪一环断了，那一环标出来，修的按钮就放在那一环上。
 * 应用按钮不在这里：它固定在页头（§7）。
 */
export function LinkChain({ links }: { links: readonly HealthLink[] }) {
  const { showHandoff } = useAppStore();

  const reveal = async () => {
    try {
      await revealClaudeConfig();
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <section aria-label="连通状态" className="panel mb-2.5 flex flex-none items-center py-3 pr-2.5 pl-[22px]">
      {links.map((link, i) => (
        <Fragment key={link.key}>
          {i > 0 && <Connector tone={worse(links[i - 1].tone, link.tone)} />}
          <Node link={link} />
        </Fragment>
      ))}
      {/* 出问题时才用得上的操作 */}
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button variant="quiet" size="icon" className="ml-3 size-8 rounded-ctl" aria-label="更多操作">
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="min-w-[15rem]">
          <DropdownMenuItem onSelect={() => void reveal()}>打开 Claude 配置目录</DropdownMenuItem>
          {isWindows() && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={showHandoff}>Windows 首次设置步骤</DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </section>
  );
}
