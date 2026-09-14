import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

import { useTheme } from "@/lib/theme";

// 2.2：toast 是面板色的浮层，类型只靠图标颜色区分 —— 层级靠明度和字重，不靠色块。
const Toaster = ({ ...props }: ToasterProps) => {
  const { dark } = useTheme();

  return (
    <Sonner
      theme={dark ? "dark" : "light"}
      className="toaster group"
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      toastOptions={{
        classNames: {
          toast: "!font-sans !text-[12.5px] !shadow-float !border-0",
          success: "[&_[data-icon]]:text-ok",
          error: "[&_[data-icon]]:text-danger",
          warning: "[&_[data-icon]]:text-accent",
          info: "[&_[data-icon]]:text-fg3",
        },
      }}
      style={
        {
          "--normal-bg": "var(--panel)",
          "--normal-text": "var(--fg)",
          "--normal-border": "var(--hair2)",
          "--border-radius": "10px",
        } as React.CSSProperties
      }
      {...props}
    />
  );
};

export { Toaster };
