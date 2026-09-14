import type { CSSProperties } from "react";

import { matchPreset } from "@/lib/presets";
import { cn } from "@/lib/utils";

import bailian from "@/assets/providers/bailian.svg";
import deepseek from "@/assets/providers/deepseek.svg";
import kimi from "@/assets/providers/kimi.svg";
import minimax from "@/assets/providers/minimax.svg";
import zhipu from "@/assets/providers/zhipu.svg";

// 品牌头像（design-2.2.md §2）：品牌色圆角方块 + 白色单色字形（单色品牌图标当遮罩用）。
// 深色模式下个别品牌色要提亮，否则纯黑 / 深蓝方块在深色面板上糊成一块。
// 没有小尺寸可读字形的品牌（mimo 的图标是两行字标，缩到 20px 就糊了）用白色首字母。
export type ProviderBrand = { color: string; dark?: string } & ({ glyph: string } | { letter: string });

const KIMI: ProviderBrand = { glyph: kimi, color: "#1c1a17", dark: "#2c2b2a" };
const BAILIAN: ProviderBrand = { glyph: bailian, color: "#e8730c" };
const BRAND_BY_PRESET: Record<string, ProviderBrand> = {
  deepseek: { glyph: deepseek, color: "#3255e8", dark: "#3f61ea" },
  "kimi-code": KIMI,
  kimi: KIMI,
  minimax: { glyph: minimax, color: "#d0405e" },
  "qwen-coding": BAILIAN,
  "qwen-token": BAILIAN,
  glm: { glyph: zhipu, color: "#5b3df5" },
  mimo: { letter: "m", color: "#ff6a00" },
};

export function presetBrand(presetId: string): ProviderBrand | undefined {
  return BRAND_BY_PRESET[presetId];
}

export function brandForUrl(url: string): ProviderBrand | undefined {
  const p = matchPreset(url);
  return p ? BRAND_BY_PRESET[p.id] : undefined;
}

/**
 * 服务商头像。认得的品牌 → 品牌色方块 + 字形；认不出的（自定义）→ 中性方块 + 首字。
 * `tone="accent"` 只给引导页的「自定义」格子用。
 */
export function ProviderAvatar({
  brand,
  letter,
  size = 26,
  tone = "neutral",
  className,
}: {
  brand?: ProviderBrand;
  letter: string;
  size?: number;
  tone?: "neutral" | "accent";
  className?: string;
}) {
  const box: CSSProperties = { width: size, height: size, borderRadius: Math.round(size * 0.27) };

  if (brand) {
    const glyph = Math.round(size * 0.6);
    const mask = "glyph" in brand ? `url("${brand.glyph}")` : "";
    return (
      <span
        aria-hidden
        className={cn(
          "flex flex-none items-center justify-center bg-(--brand) font-semibold text-white dark:bg-(--brand-dark)",
          className,
        )}
        style={
          {
            ...box,
            fontSize: Math.max(11, Math.round(size * 0.46)),
            "--brand": brand.color,
            "--brand-dark": brand.dark ?? brand.color,
          } as CSSProperties
        }
      >
        {"glyph" in brand ? (
          <span
            className="block bg-white"
            style={{
              width: glyph,
              height: glyph,
              maskImage: mask,
              WebkitMaskImage: mask,
              maskSize: "contain",
              WebkitMaskSize: "contain",
              maskRepeat: "no-repeat",
              WebkitMaskRepeat: "no-repeat",
              maskPosition: "center",
              WebkitMaskPosition: "center",
            }}
          />
        ) : (
          brand.letter
        )}
      </span>
    );
  }

  return (
    <span
      aria-hidden
      className={cn(
        "flex flex-none items-center justify-center font-semibold",
        tone === "accent"
          ? "bg-accent-weak text-accent"
          : "bg-sunken text-fg2 shadow-[inset_0_0_0_1px_var(--hair2)]",
        className,
      )}
      style={{ ...box, fontSize: Math.max(11, Math.round(size * 0.42)) }}
    >
      {letter}
    </span>
  );
}
