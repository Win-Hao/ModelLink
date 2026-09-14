import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/** 页头（design-2.2.md §6）：标题 26/600 + 一行副文案，右侧放本页的主操作。 */
export function PageHeader({
  title,
  sub,
  right,
  className,
}: {
  title: ReactNode;
  sub?: ReactNode;
  right?: ReactNode;
  className?: string;
}) {
  return (
    <header className={cn("flex flex-none items-end gap-6 pt-3 pb-6", className)}>
      <div className="min-w-0">
        <h1 className="text-title">{title}</h1>
        {sub && <p className="mt-[7px] text-[13px] tracking-[-0.003em] text-fg2">{sub}</p>}
      </div>
      {right && <div className="ml-auto flex-none">{right}</div>}
    </header>
  );
}
