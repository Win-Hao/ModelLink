#!/usr/bin/env bash
# 对当前 5678 上的代理发一组固定请求，输出保存到 $1 目录
#
# 每个 POST 都带 metadata.user_id = 用例名（原样透传到假上游），run.sh 据此按用例
# 比对捕获。2.1-A 之后部分用例与 v1 **有意不等价**，见 run.sh 顶部的 DIVERGE 表。
set -euo pipefail
OUT="$1"
mkdir -p "$OUT"
B="http://127.0.0.1:5678"

# 2.1-B 换了槽位池，两个二进制认的槽位名不同 —— 各用各的，
# 这样「第 n 个槽位 → 第 n 个模型」的比对才有意义（否则老版全部走 fallback）。
S0="${SLOT0:-claude-opus-5}"
S1="${SLOT1:-claude-sonnet-5}"
S2="${SLOT2:-claude-opus-4-8}"
S3="${SLOT3:-claude-opus-4-7}"

# 1) /v1/models
curl -s "$B/v1/models" | python3 -m json.tool --sort-keys > "$OUT/models.json"

# 2) 槽位0（te="", to_1m=auto）
curl -s -o "$OUT/r1.body" -w "%{http_code}" -X POST "$B/v1/messages" \
  -H "content-type: application/json" -H "anthropic-version: 2023-06-01" \
  -d '{"model":"'"$S0"'","max_tokens":5,"metadata":{"user_id":"slot0"},"messages":[{"role":"user","content":"hi"}]}' > "$OUT/r1.status"

# 3) 槽位0 的 [1m] 变体
curl -s -o /dev/null -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S0"'[1m]","max_tokens":5,"metadata":{"user_id":"slot0-1m"},"messages":[{"role":"user","content":"hi"}]}'

# 4) 槽位1（te=off）+ 请求自带 output_config.effort
#    ⚠️ 2.1-A §3.10 有意不等价：v1 移除 output_config 并写 disabled；
#    新版认「桌面端已指定」→ 原样透传，服务商级 off 不参与。
curl -s -o /dev/null -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S1"'","max_tokens":5,"output_config":{"effort":"low"},"metadata":{"user_id":"slot1-off-with-effort"},"messages":[{"role":"user","content":"hi"}]}'

# 5) 槽位2（te=high，请求不带 output_config → 兜底注入，与 v1 等价）
curl -s -o /dev/null -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S2"'","max_tokens":5,"metadata":{"user_id":"slot2-high"},"messages":[{"role":"user","content":"hi"}]}'

# 6) 槽位3（te=max）+ 头透传（beta/x-api-key/user-agent/自定义 anthropic-version）
curl -s -o /dev/null -X POST "$B/v1/messages" \
  -H "content-type: application/json" -H "anthropic-version: 2024-10-22" \
  -H "anthropic-beta: context-1m-2025" -H "x-api-key: caller-key" -A "ClaudeDesktop/9.9" \
  -d '{"model":"'"$S3"'","max_tokens":5,"metadata":{"user_id":"slot3-max-headers"},"messages":[{"role":"user","content":"hi"}]}'

# 7) 未匹配模型
#    ⚠️ 2.1-A §3.3 有意不等价：v1 静默回落到第一个模型并转发；新版直接 400，不打上游。
curl -s -o "$OUT/unmapped.body" -w "%{http_code}" -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"claude-nonexistent-9[1m]","max_tokens":5,"metadata":{"user_id":"unmapped"},"messages":[{"role":"user","content":"hi"}]}' > "$OUT/unmapped.status"

# 8) 其他路径 POST 透传（路径拼接行为）
curl -s -o /dev/null -X POST "$B/v1/complete" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S0"'","prompt":"x","metadata":{"user_id":"other-path"}}'

# 9) 非 POST 非 models → 404
curl -s -o "$OUT/notfound.body" -w "%{http_code}" "$B/whatever" > "$OUT/notfound.status"

# 10) 无 model 字段 → 无 target → 502 话术
curl -s -o "$OUT/nomodel.body" -w "%{http_code}" -X POST "$B/v1/messages" \
  -H "content-type: application/json" -d '{"max_tokens":5}' > "$OUT/nomodel.status"

# 11) 槽位2（te=high）+ 桌面端自带 effort/adaptive
#     ⚠️ 2.1-A §3.10 有意不等价：v1 把两者都踩掉；新版原样透传。
curl -s -o "$OUT/passthrough.body" -w "%{http_code}" -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S2"'","max_tokens":5,"output_config":{"effort":"low"},"thinking":{"type":"adaptive"},"metadata":{"user_id":"slot2-passthrough"},"messages":[{"role":"user","content":"hi"}]}' > "$OUT/passthrough.status"

# ---- 以下用例会污染服务商能力缓存（§3.11.1），必须放在最后 ----

# 12) 上游拒收 output_config → 代理层整流重试
#     ⚠️ 2.1-A §3.11.1 有意不等价：v1 原样吐 400；新版去掉 output_config 重试成功。
curl -s -o "$OUT/rectify.body" -w "%{http_code}" -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S0"'","max_tokens":5,"output_config":{"effort":"max"},"metadata":{"user_id":"effort-reject-1"},"messages":[{"role":"user","content":"hi"}]}' > "$OUT/rectify.status"

# 13) 同一服务商+模型的下一发：能力缓存命中，effort 直接不发（只有一次往返）
curl -s -o /dev/null -X POST "$B/v1/messages" \
  -H "content-type: application/json" \
  -d '{"model":"'"$S0"'","max_tokens":5,"output_config":{"effort":"max"},"metadata":{"user_id":"effort-reject-2"},"messages":[{"role":"user","content":"hi"}]}'

sleep 0.3
echo "driven: $OUT"
