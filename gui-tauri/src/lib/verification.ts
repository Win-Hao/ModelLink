import type { Provider } from "@/lib/ipc";

// 「测试连接」的结论要常驻（design-2.2.md §6.2）：2.0 测完弹个 toast 就没了，
// 用户下次打开完全看不出这个密钥上次通没通。
//
// 按「地址 + 密钥」记 —— 换了密钥就是没验证过。localStorage 里只存指纹，不存密钥原文。

/** 一次「测试连接」的结论。 */
export type Verification = { ok: boolean; at: number; message: string };

const STORAGE_KEY = "modellink.verifications";

/** cyrb53：53 位字符串哈希，够区分就行，不做安全用途。 */
function cyrb53(s: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < s.length; i++) {
    const ch = s.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36);
}

export function verificationKey(p: Pick<Provider, "target_url" | "api_key">): string {
  return cyrb53(`${p.target_url.trim()}\n${p.api_key.trim()}`);
}

export function loadVerifications(): Record<string, Verification> {
  try {
    const v = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

/** 落盘时顺手丢掉已经不对应任何服务商的记录（换掉的旧密钥不留痕）。 */
export function saveVerifications(all: Record<string, Verification>, providers: Provider[]) {
  const live = new Set(providers.map(verificationKey));
  const kept = Object.fromEntries(Object.entries(all).filter(([k]) => live.has(k)));
  localStorage.setItem(STORAGE_KEY, JSON.stringify(kept));
}

/** 后端的测试话术是英文（与 v1 一致），换成用户看得懂的说法；上游自己的报错原样保留。 */
export function verificationText(message: string): string {
  if (message.startsWith("Cannot connect")) return "连不上这个地址";
  if (message === "Connection timed out.") return "连接超时";
  if (message.startsWith("URL must start with")) return "地址要以 http:// 或 https:// 开头";
  const http = message.match(/^HTTP (\d{3})$/);
  if (http) {
    const code = Number(http[1]);
    if (code === 401 || code === 403) return `密钥不对（HTTP ${code}）`;
    if (code === 404) return "地址不对（HTTP 404）";
    return `服务商返回 HTTP ${code}`;
  }
  return message.replace(/^Error: /, "");
}
