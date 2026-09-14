#!/usr/bin/env node
/**
 * 生成 src/lib/modelsSnapshot.ts —— models.dev 模型清单的发版快照。
 *
 * 为什么要有：模型名输入框的补全优先用运行时同步的数据，但用户首次打开、
 * 断网、或 models.dev 不可达时拿不到。手写清单会过期（实测 Kimi Code 那条
 * 落后两代），所以改成发版时自动生成，兜底永远是「发版当天的最新」。
 *
 * 用法：npm run sync-models      （发版前跑一次，把改动一起提交）
 */
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// 与 src-tauri/src/models_dev.rs::known_provider_ids() 保持一致
const KNOWN = [
  "kimi-for-coding",
  "moonshotai-cn",
  "deepseek",
  "minimax-cn",
  "minimax",
  "alibaba-coding-plan-cn",
  "alibaba-token-plan-cn",
  "alibaba-cn",
  "zhipuai",
  "xiaomi",
  "xiaomi-token-plan-cn",
];

const res = await fetch("https://models.dev/api.json", { signal: AbortSignal.timeout(60_000) });
if (!res.ok) throw new Error(`models.dev 返回 HTTP ${res.status}`);
const data = await res.json();

const out = {};
for (const pid of KNOWN) {
  const models = data[pid]?.models;
  if (!models) {
    console.warn(`  ⚠ ${pid}: models.dev 里没有这家，跳过`);
    continue;
  }
  // 与后端 parse_model_ids 同一套排序：发布日期新→旧，同日期按 ID 升序
  out[pid] = Object.entries(models)
    .map(([id, m]) => [id, m.release_date ?? ""])
    .sort((a, b) => b[1].localeCompare(a[1]) || a[0].localeCompare(b[0]))
    .map(([id]) => id);
  console.log(`  ${pid.padEnd(24)} ${String(out[pid].length).padStart(2)} 个，最新: ${out[pid][0]}`);
}

const date = new Date().toISOString().slice(0, 10);
const body = `// 由 scripts/sync-models-snapshot.mjs 生成，请勿手改。
// 数据源：https://models.dev/api.json　抓取日期：${date}
//
// 这是**兜底**清单：模型名输入框优先用运行时同步来的数据，拉不到时才用这份。
// 发版前跑 \`npm run sync-models\` 刷新，把改动一起提交。

/** 抓取日期，界面上标注「截至 X」用。 */
export const MODELS_SNAPSHOT_DATE = "${date}";

/** models.dev 服务商 ID → 模型 ID（发布日期新→旧）。 */
export const MODELS_SNAPSHOT: Record<string, string[]> = ${JSON.stringify(out, null, 2)};
`;

const here = dirname(fileURLToPath(import.meta.url));
const target = join(here, "..", "src", "lib", "modelsSnapshot.ts");
writeFileSync(target, body);
console.log(`\n已写入 ${target}（${(body.length / 1024).toFixed(1)} KB，抓取日期 ${date}）`);
