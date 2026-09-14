import { ProviderAvatar, presetBrand } from "@/components/ProviderAvatar";
import { PRESETS, presetHost, type Preset } from "@/lib/presets";

// 预设短名（网格格子里的文案；创建服务商仍用 PRESETS 完整数据）
const SHORT_NAMES: Record<string, string> = {
  deepseek: "DeepSeek",
  "kimi-code": "Kimi Code",
  kimi: "Kimi 开放平台",
  minimax: "MiniMax",
  "qwen-coding": "百炼 Coding Plan",
  "qwen-token": "百炼 Token Plan",
  glm: "GLM（智谱）",
  mimo: "mimo",
};

// 同一家有两条产品线、图标一模一样时，格子上把区别写出来（取自预设全名，不另编）。
// 跟着视频配置的人，要能一眼认出该点哪一个。
const PLAN_NOTES: Record<string, string> = {
  "kimi-code": "订阅制",
  kimi: "按量付费",
};

const tile = "flex min-w-0 items-center gap-[11px] rounded-md px-[15px] py-3.5 text-left transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/40";

/**
 * 3×3 预设网格（8 个预设 + 自定义），用于首启引导页与「添加服务商」弹窗。
 * `configured` 里的预设已经配过：格子上标「已添加」，点它是跳过去加模型，不是再建一个。
 */
export function PresetGrid({
  onPick,
  configured,
}: {
  onPick: (p: Preset | "custom") => void;
  configured?: ReadonlySet<string>;
}) {
  return (
    <div className="grid grid-cols-3 gap-2.5">
      {PRESETS.map((p) => {
        const name = SHORT_NAMES[p.id] ?? p.name;
        return (
          <button key={p.id} onClick={() => onPick(p)} className={`${tile} bg-panel shadow-panel hover:bg-sunken`}>
            <ProviderAvatar brand={presetBrand(p.id)} letter={name[0]} size={32} />
            <span className="min-w-0">
              <span className="block text-[13px] font-semibold tracking-[-0.012em]">{name}</span>
              {configured?.has(p.id) ? (
                <span className="mt-[3px] block truncate text-[12px] text-fg3">已添加 · 点这里加模型</span>
              ) : PLAN_NOTES[p.id] ? (
                <span className="mt-[3px] block truncate text-[12px] text-fg3">
                  {PLAN_NOTES[p.id]} · <span className="mono">{presetHost(p)}</span>
                </span>
              ) : (
                <span className="mono mt-[3px] block truncate text-[12px] text-fg3">{presetHost(p)}</span>
              )}
            </span>
          </button>
        );
      })}
      <button onClick={() => onPick("custom")} className={`${tile} inset-ring inset-ring-hair2 hover:bg-panel`}>
        <ProviderAvatar letter="?" size={32} tone="accent" />
        <span className="min-w-0">
          <span className="block text-[13px] font-semibold tracking-[-0.012em]">自定义</span>
          <span className="mt-[3px] block truncate text-[12px] text-fg3">手动填写地址</span>
        </span>
      </button>
    </div>
  );
}
