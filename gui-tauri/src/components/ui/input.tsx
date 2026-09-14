import * as React from "react"

import { cn } from "@/lib/utils"

// 输入框一律等宽：这个应用里能填的全是技术值（URL、密钥、端口、模型名）。
function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "h-[34px] w-full min-w-0 rounded-ctl bg-sunken px-[11px] font-mono text-[12.5px] text-fg shadow-[inset_0_0_0_1px_var(--hair2)] transition-[box-shadow] outline-none placeholder:text-fg3 disabled:cursor-not-allowed disabled:opacity-50",
        "focus-visible:shadow-[inset_0_0_0_1px_var(--accent),0_0_0_3px_var(--accent-weak)]",
        "aria-invalid:shadow-[inset_0_0_0_1px_var(--danger)]",
        className
      )}
      {...props}
    />
  )
}

export { Input }
