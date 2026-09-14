import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// index.css 里自定义的字阶 / 圆角 / 阴影 token 要登记给 tailwind-merge：
// 不登记的话它把 `text-label` 当成文字颜色，和 `text-fg3` 一起出现时会把字号类吞掉。
const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [{ text: ["title", "stat", "heading", "data", "body", "label"] }],
      rounded: [{ rounded: ["ctl"] }],
      shadow: [{ shadow: ["panel", "float"] }],
    },
  },
});

/** 合并 className：clsx 拼接 + tailwind-merge 去冲突（shadcn 约定） */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
