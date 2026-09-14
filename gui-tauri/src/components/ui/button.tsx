import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

// 2.2 控件规格（design-2.2.md §4）：主按钮 38px · 小按钮 30px · 图标按钮 30×30。
// 描边一律用内阴影画（与面板同一套发丝线），虚线「添加」例外 —— 阴影画不出虚线。
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 font-semibold tracking-[-0.008em] whitespace-nowrap transition-[background-color,color,box-shadow,opacity] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
  {
    variants: {
      variant: {
        default: "bg-accent text-on-accent hover:bg-accent/90",
        ghost:
          "bg-transparent font-medium text-fg shadow-[inset_0_0_0_1px_var(--hair2)] hover:bg-hair",
        danger: "bg-transparent font-medium text-danger hover:bg-danger/10",
        // 确认删除这类不可逆操作的实心按钮；on-accent 是「实心色块上的字」，深色下是深字
        destructive: "bg-danger text-on-accent hover:bg-danger/90",
        dashed:
          "border border-dashed border-hair2 bg-transparent font-medium text-fg3 hover:border-fg3 hover:text-fg2",
        quiet: "bg-transparent font-medium text-fg3 hover:bg-hair hover:text-fg",
      },
      size: {
        default: "h-[38px] rounded-md px-[18px] text-[13.5px]",
        sm: "h-[30px] rounded-ctl px-3 text-[12.5px]",
        icon: "size-[30px] rounded-[7px] text-fg2",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
