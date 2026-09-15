import { useQuery } from "@tanstack/react-query";

import { appliedState, proxyStatus, type Config, type Provider } from "@/lib/ipc";
import { flattenModels, providerDisplayName } from "@/lib/presets";
import { useAppStore } from "@/lib/store";
import { verificationText } from "@/lib/verification";

// ============================================================
// 连通链（design-2.2.md §6.1）：Claude Desktop → ModelLink → 服务商，三环各自的状态，
// 以及页头的应用按钮、修的按钮。概览页的链、页头按钮、设置页的一键排查都读这一份，
// 免得一处说「已生效」、另一处说「连不上」。
// ============================================================

/** ok 正常 · idle 没结论（读取中 / 没测过）· attention 要你动手 · bad 坏了 */
export type Tone = "ok" | "idle" | "attention" | "bad";

export type HealthLink = {
  key: "claude" | "proxy" | "provider";
  title: string;
  tone: Tone;
  /** 一句话状态 */
  text: string;
  /** 跟在状态后面的技术值（端口），等宽显示 */
  mono?: string;
  /** 悬停时的补充说明 */
  hint?: string;
  /** 点这一环去修 */
  fix?: { label: string; run: () => void };
};

/** 页头的「应用」按钮（design-2.2.md §7）：一直在，轻重跟着状态走。 */
export type ApplyAction =
  | { kind: "busy"; label: string }
  /** 有服务商没填完：应用一定失败，按钮灰掉，旁边写缺什么 */
  | { kind: "blocked"; label: string; reason: string }
  /** note 是按钮旁边那句「会重启 Claude」 */
  | { kind: "ready"; tone: "primary" | "quiet"; label: string; note: string; run: () => void };

/** 页头那一行话 */
export type StatusLine = { tone: Tone; text: string };

const RANK: Record<Tone, number> = { ok: 0, idle: 1, attention: 2, bad: 3 };
export const worse = (a: Tone, b: Tone): Tone => (RANK[a] >= RANK[b] ? a : b);

type Gap = { index: number; what: "url" | "urlScheme" | "key" | "models" | "modelName"; row: number };

/** 按「应用」的校验顺序找第一处没填完的（与 gateway.rs apply_to_claude_desktop 同序）。 */
export function firstGap(config: Config | null): Gap | null {
  if (!config) return null;
  for (const [index, p] of config.providers.entries()) {
    if (!p.target_url) return { index, what: "url", row: -1 };
    if (!/^https?:\/\//.test(p.target_url)) return { index, what: "urlScheme", row: -1 };
    if (!p.api_key) return { index, what: "key", row: -1 };
    if (p.models.length === 0) return { index, what: "models", row: -1 };
    const row = p.models.findIndex((m) => !m.name);
    if (row >= 0) return { index, what: "modelName", row };
  }
  return null;
}

const GAP_TEXT: Record<Gap["what"], [string, string]> = {
  url: ["还没填 API 地址", "去填 API 地址"],
  urlScheme: ["的 API 地址要以 https:// 开头", "去改 API 地址"],
  key: ["还没填 API 密钥", "去填 API 密钥"],
  models: ["还没有模型", "去加模型"],
  modelName: ["有一行模型没选", "去选模型"],
};

/** 应用按钮灰掉时旁边那句：先做什么才能应用 */
const GAP_REASON: Record<Gap["what"], (name: string) => string> = {
  url: (n) => `先填好「${n}」的 API 地址`,
  urlScheme: (n) => `先把「${n}」的 API 地址改成 https:// 开头`,
  key: (n) => `先填好「${n}」的 API 密钥`,
  models: (n) => `先给「${n}」加一个模型`,
  modelName: (n) => `先选好「${n}」没选的那行模型`,
};

export function useHealth() {
  const store = useAppStore();
  const {
    draft,
    applyState,
    applyError,
    apply,
    pendingCount,
    pendingApply: pa,
    configChanged,
    poolUpgrade,
    savedLive,
    verificationFor,
    isTesting,
    testProviders,
    gotoPort,
    gotoProvider,
    gotoProviderField,
    gotoModelPick,
    setPickerOpen,
  } = store;
  const statusQ = useQuery({ queryKey: ["proxy-status"], queryFn: proxyStatus });
  const appliedQ = useQuery({ queryKey: ["applied-state"], queryFn: appliedState });

  const providers = draft?.providers ?? [];
  const neverApplied = !draft?.last_applied_at;
  const nameOf = (i: number) => providerDisplayName(providers[i]?.target_url ?? "", i);
  const st = statusQ.data;
  const proxyDown = !!st && !st.running;
  const gap = firstGap(draft);

  const gapFix = (g: Gap) => ({
    label: GAP_TEXT[g.what][1],
    run: () =>
      g.what === "url" || g.what === "urlScheme"
        ? gotoProviderField(g.index, "url")
        : g.what === "key"
          ? gotoProviderField(g.index, "key")
          : g.what === "modelName"
            ? gotoModelPick(g.index, g.row)
            : gotoProvider(g.index),
  });
  const gapText = (g: Gap) => `「${nameOf(g.index)}」${GAP_TEXT[g.what][0]}`;

  // Claude 的设置页里改得动、改掉就坏事的两项（后端 pending_apply 查的）。模型列表照样对得上，不说就没人知道
  const drift = [
    pa?.egress && { what: "允许联网的域名", effect: "Cowork 和 Code 里抓网页、装包会失败" },
    pa?.identity && { what: "模型身份说明", effect: "模型可能会说自己是 Claude" },
  ].filter((d): d is { what: string; effect: string } => !!d);

  // ---- 为什么要应用（同一个原因，链上短说、页头长说，悬停看细节） ----
  let applyWhy: { short: string; long: string; hint?: string } | null = null;
  if (applyState === "dirty") {
    if (neverApplied) {
      applyWhy = { short: "还没接入", long: "配好了，应用到 Claude Desktop 就能用" };
    } else if (poolUpgrade) {
      applyWhy = { short: "新版要重新应用一次", long: "新版为你的模型打开了 Claude 里的思考深度选择，应用一次就能用" };
    } else if (pa?.port_changed) {
      applyWhy = { short: "端口换了，还没应用", long: "端口换了，应用一次 Claude 才会连到新端口" };
    } else if (pendingCount > 0) {
      applyWhy = configChanged
        ? { short: `有 ${pendingCount} 处改动还没应用`, long: `有 ${pendingCount} 处改动还没应用到 Claude Desktop` }
        : { short: `有 ${pendingCount} 处和这里对不上`, long: `Claude Desktop 里有 ${pendingCount} 处和这里对不上` };
    } else if (drift.length > 0) {
      applyWhy = {
        short: `有 ${drift.length} 处和这里对不上`,
        long:
          drift.length === 1
            ? `Claude 里的${drift[0].what}被改了，${drift[0].effect}`
            : `Claude Desktop 里有 ${drift.length} 处被改了，应用一次就能改回来`,
      };
    } else if (pa?.pricing) {
      applyWhy = { short: "费率有更新，还没应用", long: "模型费率有更新，应用后 Claude 里的费用才按新价算" };
    } else {
      applyWhy = { short: "有改动还没应用", long: "配置改过，还没应用到 Claude Desktop" };
    }
    // 同时有别的原因要应用时，被改掉的那几项也写进悬停说明，不被盖住
    if (drift.length > 0) {
      applyWhy.hint = [...drift.map((d) => `${d.what}被改了：${d.effect}`), "应用一次就能改回来。"].join("\n");
    }
  }

  // ---- 三环 ----
  const flat = draft ? flattenModels(draft) : [];
  const a = appliedQ.data;

  let claude: HealthLink;
  if (applyState === "applying") {
    claude = { key: "claude", title: "Claude Desktop", tone: "attention", text: "正在写入，Claude 会自动重启" };
  } else if (applyState === "error") {
    claude = { key: "claude", title: "Claude Desktop", tone: "bad", text: "上次应用没成功", hint: applyError ?? undefined };
  } else if (!a || !draft) {
    claude = { key: "claude", title: "Claude Desktop", tone: "idle", text: "正在读取…" };
  } else if (!a.found && neverApplied) {
    claude = { key: "claude", title: "Claude Desktop", tone: "attention", text: "还没接入" };
  } else if (!a.found) {
    claude = {
      key: "claude",
      title: "Claude Desktop",
      tone: "bad",
      text: "没找到它的配置",
      hint: "Claude Desktop 可能没装，或者配置被删了。应用一次会重新写进去。",
    };
  } else if (pa?.gateway && !pa.port_changed) {
    claude = {
      key: "claude",
      title: "Claude Desktop",
      tone: "bad",
      text: "没接到 ModelLink",
      hint: "Claude Desktop 的第三方推理配置被改过，现在没有指向 ModelLink。应用一次就能接回来。",
    };
  } else if (applyWhy) {
    claude = { key: "claude", title: "Claude Desktop", tone: "attention", text: applyWhy.short, hint: applyWhy.hint };
  } else {
    claude = { key: "claude", title: "Claude Desktop", tone: "ok", text: `已接入 · ${flat.length} 个模型` };
  }

  const proxy: HealthLink = !st
    ? { key: "proxy", title: "ModelLink", tone: "idle", text: "正在读取…" }
    : st.running
      ? { key: "proxy", title: "ModelLink", tone: "ok", text: "在运行", mono: `127.0.0.1:${st.port}` }
      : {
          key: "proxy",
          title: "ModelLink",
          tone: "bad",
          text: `没在运行，端口 ${st.port} 被别的程序占了`,
          hint: "Claude 发来的请求到不了 ModelLink。换一个没人用的端口，再应用一次。",
          fix: { label: "换一个端口", run: gotoPort },
        };

  let provider: HealthLink;
  const results = providers.map((p: Provider) => verificationFor(p));
  const failed = results.flatMap((v, i) => (v && !v.ok ? [i] : []));
  const untested = providers.flatMap((p, i) => (!results[i] && p.api_key ? [i] : []));
  if (providers.length === 0) {
    provider = {
      key: "provider",
      title: "服务商",
      tone: "idle",
      text: "还没有服务商",
      fix: { label: "添加服务商", run: () => setPickerOpen(true) },
    };
  } else if (gap) {
    provider = { key: "provider", title: "服务商", tone: neverApplied ? "attention" : "bad", text: gapText(gap), fix: gapFix(gap) };
  } else if (providers.some((p) => isTesting(p))) {
    provider = { key: "provider", title: "服务商", tone: "idle", text: "正在测试连接…" };
  } else if (failed.length > 0) {
    provider = {
      key: "provider",
      title: "服务商",
      tone: "bad",
      text: failed.length === 1 ? `「${nameOf(failed[0])}」连不上` : `${failed.length} 家连不上`,
      hint: failed.map((i) => `${nameOf(i)}：${verificationText(results[i]!.message)}`).join("\n"),
      fix: { label: "去看看", run: () => gotoProvider(failed[0]) },
    };
  } else if (untested.length > 0) {
    provider = {
      key: "provider",
      title: "服务商",
      tone: "idle",
      text: untested.length === providers.length ? "还没测试过连接" : `${untested.length} 家还没测试过`,
      fix: { label: "测试连接", run: () => void testProviders(untested) },
    };
  } else {
    provider = {
      key: "provider",
      title: "服务商",
      tone: "ok",
      text: providers.length === 1 ? `${nameOf(0)} 已连通` : `${providers.length} 家都已连通`,
    };
  }

  // ---- 页头的「应用」按钮：一直在 ----
  // 原先只在要应用时出现，平时收在 ⋯ 菜单里；有服务商没填完时页头换成「去填 API 密钥」，
  // 看着「有 N 处改动还没应用」却找不到应用按钮（2026-09-15 作者实测）
  let applyButton: ApplyAction | null = null;
  if (providers.length > 0) {
    if (applyState === "applying") applyButton = { kind: "busy", label: "正在重启 Claude…" };
    // 没填完时应用一定失败 —— 按钮亮着的话，点了报错，配的按钮却是「重试」
    else if (gap) applyButton = { kind: "blocked", label: "应用到 Claude Desktop", reason: GAP_REASON[gap.what](nameOf(gap.index)) };
    else if (applyState === "error") applyButton = { kind: "ready", tone: "primary", label: "重试", note: "会重启 Claude Desktop", run: apply };
    // 代理没在跑时要先换端口（顶栏和链上都在说），应用不抢这个位置
    else if (applyState === "dirty")
      applyButton = { kind: "ready", tone: proxyDown ? "quiet" : "primary", label: "应用到 Claude Desktop", note: "会重启 Claude Desktop", run: apply };
    else applyButton = { kind: "ready", tone: "quiet", label: "重新应用", note: "会重启 Claude", run: apply };
  }

  // ---- 修的按钮：概览页放在出问题的那一环上；服务商页没有链，放在应用按钮旁边 ----
  const fix = proxyDown ? proxy.fix! : gap ? gapFix(gap) : null;

  // ---- 页头那一行话（服务商页；概览页由链来说） ----
  let line: StatusLine;
  if (applyState === "applying") line = { tone: "idle", text: "Claude Desktop 会自动重启" };
  else if (proxyDown) line = { tone: "bad", text: "ModelLink 没在运行，Claude 现在连不上" };
  // 和链上服务商那一环同一句：没填完就说，不管要不要应用 —— 应用过之后把密钥删了，请求照样会失败
  else if (gap) line = { tone: neverApplied ? "attention" : "bad", text: gapText(gap) };
  else if (applyState === "error") line = { tone: "bad", text: `应用失败：${applyError}` };
  else if (applyWhy) line = { tone: "attention", text: applyWhy.long };
  else if (savedLive) line = { tone: "ok", text: "已保存，立即生效" };
  else line = { tone: "ok", text: "配置已生效" };

  return { links: [claude, proxy, provider] as const, apply: applyButton, fix, line, proxyDown, gap };
}
