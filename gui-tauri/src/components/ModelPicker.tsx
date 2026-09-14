import { useEffect, useId, useRef, useState } from "react";
import { Check, ChevronDown, CornerDownLeft, Search } from "lucide-react";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { AvailableModel } from "@/lib/ipc";
import { formatContext } from "@/lib/presets";
import { cn } from "@/lib/utils";

type Item = AvailableModel & { custom?: boolean };

/**
 * 模型选择器（design-2.2.md §6.2）：从这家服务商的模型清单里挑，不是自由文本框。
 * 清单兜不住的（服务商刚上新）可以直接输入 —— 输入框里敲的名字会作为第一项「使用「…」」。
 */
export function ModelPicker({
  label,
  value,
  options,
  caption,
  showFooter,
  openSignal = 0,
  onPick,
}: {
  /** 读屏用的名字（触发器上只有模型名，读出来不知道是什么） */
  label: string;
  value: string;
  options: AvailableModel[];
  /** 清单上方一行：这份清单从哪来 */
  caption: string;
  /** 「找不到？直接输入」那句 —— 有清单时才需要 */
  showFooter: boolean;
  /** 变成新的非零值时自动展开（刚加出来的空行、从「添加服务商」跳过来加模型） */
  openSignal?: number;
  onPick: (id: string, context: number | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const q = query.trim();
  const lower = q.toLowerCase();
  const filtered = q ? options.filter((o) => o.id.toLowerCase().includes(lower)) : options;
  const typedIsNew = q !== "" && !options.some((o) => o.id.toLowerCase() === lower);
  const items: Item[] = [...(typedIsNew ? [{ id: q, context: null, custom: true }] : []), ...filtered];

  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-index="${active}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const changeOpen = (next: boolean) => {
    setOpen(next);
    if (next) {
      setQuery("");
      setActive(Math.max(0, options.findIndex((o) => o.id === value)));
    }
  };

  useEffect(() => {
    if (openSignal) changeOpen(true);
    // 只跟着信号走；changeOpen 每次渲染都是新函数，放进依赖会反复打开
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openSignal]);

  const pick = (item: Item) => {
    onPick(item.id, item.context);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={changeOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          aria-controls={listId}
          className="group flex h-[34px] w-[252px] items-center gap-2 rounded-ctl bg-sunken px-2.5 text-left shadow-[inset_0_0_0_1px_var(--hair2)] transition-shadow outline-none focus-visible:shadow-[inset_0_0_0_1px_var(--accent),0_0_0_3px_var(--accent-weak)] data-[state=open]:shadow-[inset_0_0_0_1px_var(--accent)]"
        >
          <span
            className={cn(
              "mono min-w-0 flex-1 truncate text-[13px] font-medium tracking-[-0.01em]",
              !value && "font-sans font-normal text-fg3",
            )}
          >
            {value || "选择模型"}
          </span>
          <ChevronDown className="size-3 flex-none text-fg3 transition-transform duration-200 ease-out-expo group-data-[state=open]:rotate-180" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-[330px]">
        <div className="flex items-center gap-[7px] border-b border-hair px-[9px] pt-0.5 pb-1.5">
          <Search className="size-3.5 flex-none text-fg3" />
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((a) => Math.min(items.length - 1, a + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => Math.max(0, a - 1));
              } else if (e.key === "Enter" && items[active]) {
                e.preventDefault();
                pick(items[active]);
              }
            }}
            placeholder="搜索，或直接输入模型名"
            aria-label="搜索模型，或直接输入模型名"
            aria-controls={listId}
            aria-activedescendant={items[active] ? `${listId}-${active}` : undefined}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            className="mono h-7 min-w-0 flex-1 bg-transparent text-[12.5px] text-fg outline-none placeholder:font-sans placeholder:text-fg3"
          />
        </div>
        <p className="px-[9px] pt-2 pb-1 text-[11.5px] leading-[1.5] text-fg3">{caption}</p>
        <div ref={listRef} id={listId} role="listbox" className="max-h-[232px] overflow-y-auto">
          {items.map((it, i) => (
            <div
              key={it.custom ? `custom:${it.id}` : it.id}
              id={`${listId}-${i}`}
              data-index={i}
              role="option"
              aria-selected={i === active}
              onMouseMove={() => setActive(i)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(it)}
              className={cn(
                "flex h-8 cursor-default items-center gap-[9px] rounded-[7px] px-[9px] text-[12.5px]",
                !it.custom && it.id === value && "bg-accent-weak",
                i === active && "bg-hair",
              )}
            >
              {it.custom ? (
                <>
                  <CornerDownLeft className="size-3.5 flex-none text-fg3" />
                  <span className="min-w-0 flex-1 truncate">
                    使用「<span className="mono font-medium">{it.id}</span>」
                  </span>
                </>
              ) : (
                <>
                  <span className="mono min-w-0 flex-1 truncate font-medium">{it.id}</span>
                  <span className="mono flex-none text-[11.5px] text-fg3">
                    {formatContext(it.context ?? undefined)}
                  </span>
                  <span className="flex w-3.5 flex-none justify-center text-accent">
                    {it.id === value && <Check className="size-3.5" />}
                  </span>
                </>
              )}
            </div>
          ))}
        </div>
        {showFooter && (
          <p className="mt-1 border-t border-hair px-[9px] pt-2 pb-[5px] text-[11.5px] leading-[1.5] text-fg3">
            找不到？直接输入模型名。服务商刚上新时，清单可能还没同步到
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
