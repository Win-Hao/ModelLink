#!/usr/bin/env bash
# 新旧代理逐字节等价回归（docs/gui-rebuild-tauri.md §9/§10 的抓包对比项）。
#
# 原理：temp HOME 隔离真实配置 → 假上游（upstream.py）捕获代理转发的
# method/path/headers/body → 对新旧两个二进制发同一组请求（drive.sh）→ 按用例 diff。
# 覆盖：/v1/models 格式、thinking 态注入、[1m] 变体、头透传、
# 路径拼接、404/502 话术、Claude-3p 网关写入、LaunchAgent 迁移。
#
# 2.1-A 起「与 v1 逐字节等价」不再是全量目标：DIVERGE 表里的用例是**有意**不等价的
# 行为改动，对它们改为断言新行为（见下方 python 段）。表外的用例仍须逐字节一致。
#
# 2.1-B 起槽位池换成了 claude-opus-5 等（§2.1），所以 /v1/models 与网关 inferenceModels
# 里的槽位名与 v1 必然不同 —— 比对时按顺序对齐，只校验「第 n 个槽位映射到第 n 个模型」。
#
# 用法：OLD_BIN=<v1 可执行> NEW_BIN=<v2 可执行> bash regression/run.sh
#   v1 二进制取自仓库根 ModelLink-macOS.zip（或 GitHub v1.2.0 release）：
#     unzip -o ModelLink-macOS.zip -d /tmp/mlv1 && xattr -cr /tmp/mlv1
#     OLD_BIN=/tmp/mlv1/ModelLink.app/Contents/MacOS/modellink
#   （带 com.apple.quarantine 的裸二进制会被 Gatekeeper 静默击杀，xattr -c 必做）
set -euo pipefail
EQ="$(mktemp -d /tmp/modellink-equiv.XXXXXX)"
HERE="$(cd "$(dirname "$0")" && pwd)"
OLD_BIN="${OLD_BIN:?set OLD_BIN to the v1 binary}"
NEW_BIN="${NEW_BIN:?set NEW_BIN to the v2 binary}"

mk_home() {
  rm -rf "$1"; mkdir -p "$1/.claude-model-proxy" "$1/Library/LaunchAgents"
  cat > "$1/.claude-model-proxy/config.json" <<'EOF'
{
  "providers": [
    {"target_url": "http://127.0.0.1:9999", "api_key": "test-key", "models": [{"name": "real-a", "to_1m": "auto"}], "thinking_effort": ""},
    {"target_url": "http://127.0.0.1:9999/sub", "api_key": "test-key-2", "models": [{"name": "real-b", "to_1m": ""}], "thinking_effort": "off"},
    {"target_url": "http://127.0.0.1:9999", "api_key": "test-key-3", "models": [{"name": "real-c", "to_1m": ""}], "thinking_effort": "high"},
    {"target_url": "http://127.0.0.1:9999", "api_key": "test-key-4", "models": [{"name": "real-d", "to_1m": "auto"}], "thinking_effort": "max"},
    {"target_url": "http://127.0.0.1:9998", "api_key": "test-key-e", "models": [{"name": "real-e", "to_1m": ""}], "thinking_effort": ""},
    {"target_url": "http://127.0.0.1:9999", "api_key": "test-key-5", "models": [{"name": "over-0", "to_1m": ""}, {"name": "over-1", "to_1m": ""}, {"name": "over-2", "to_1m": ""}, {"name": "over-3", "to_1m": ""}, {"name": "over-4", "to_1m": ""}, {"name": "over-5", "to_1m": ""}, {"name": "over-6", "to_1m": ""}, {"name": "over-7", "to_1m": ""}, {"name": "over-8", "to_1m": ""}, {"name": "over-9", "to_1m": ""}, {"name": "over-10", "to_1m": ""}, {"name": "over-11", "to_1m": ""}, {"name": "over-12", "to_1m": ""}, {"name": "over-13", "to_1m": ""}, {"name": "over-14", "to_1m": ""}, {"name": "over-15", "to_1m": ""}, {"name": "over-16", "to_1m": ""}, {"name": "over-17", "to_1m": ""}], "thinking_effort": ""}
  ],
  "pricing_auto_sync": false
}
EOF
}

wait_port() {
  for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:5678/v1/models" && return 0; sleep 0.25; done
  echo "proxy not ready" >&2; return 1
}

run_one() { # $1=label $2=binary $3=home
  local label="$1" bin="$2" home="$3"
  rm -f "$EQ/cap-$label.jsonl"
  python3 "$HERE/upstream.py" "$EQ/cap-$label.jsonl" 9999 & local up=$!
  # 第二个假上游：给「整流组合路径」用例专用，免得和别的用例共享服务商能力缓存
  python3 "$HERE/upstream.py" "$EQ/cap-$label.jsonl" 9998 & local up2=$!
  sleep 0.5
  HOME="$home" "$bin" >/dev/null 2>"$EQ/app-$label.log" & local app=$!
  wait_port
  if [ "$label" = "old" ]; then
    # v1 认的是 2.0 的 legacy 槽位池
    SLOT0=claude-3-opus-latest SLOT1=claude-3-5-sonnet-latest \
    SLOT2=claude-3-sonnet-20240229 SLOT3=claude-3-haiku-20240307 \
    SLOT4=claude-3-5-haiku-latest \
      bash "$HERE/drive.sh" "$EQ/out-$label" >/dev/null
  else
    bash "$HERE/drive.sh" "$EQ/out-$label" >/dev/null
  fi
  kill "$app" 2>/dev/null; wait "$app" 2>/dev/null || true
  kill "$up" 2>/dev/null; wait "$up" 2>/dev/null || true
  kill "$up2" 2>/dev/null; wait "$up2" 2>/dev/null || true
  sleep 0.5
}

# 防呆：5678/9999 必须空闲（正在跑的 ModelLink 会让 wait_port 等到错误对象）
for p in 5678 9999 9998; do
  if lsof -nP -i ":$p" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "✗ 端口 $p 被占用（先退出正在运行的 ModelLink / 其他占用者再跑）" >&2
    exit 1
  fi
done

mk_home "$EQ/home-old"
mk_home "$EQ/home-new"
# 在新版 home 里种一个旧 LaunchAgent，验证迁移（红线 #5）
printf 'fake-v1-plist' > "$EQ/home-new/Library/LaunchAgents/com.modellink.plist"

echo "=== run OLD ($OLD_BIN) ==="
run_one old "$OLD_BIN" "$EQ/home-old"
echo "=== run NEW ($NEW_BIN) ==="
run_one new "$NEW_BIN" "$EQ/home-new"

fail=0
echo
echo "=== 上游捕获：表外用例逐字节等价 + DIVERGE 用例断言新行为 ==="
[ -s "$EQ/cap-old.jsonl" ] || { echo "✗ 老版捕获为空"; fail=1; }
if python3 - "$EQ" <<'PY'
import json, sys, pathlib

eq = pathlib.Path(sys.argv[1])

# 2.1-A 有意不等价的用例 → 对新版断言新行为，不与 v1 比对
DIVERGE = {
    "slot1-off-with-effort": "§3.10 桌面端已指定 effort，服务商级 off 不参与",
    "slot2-high":            "§3.10 兜底注入改发 adaptive，不再写死 budget 8192",
    "slot3-max-headers":     "§3.10 同上（顺带验头透传）",
    "slot2-passthrough":     "§3.10 effort + adaptive 原样透传",
    "unmapped":              "§3.3 未映射槽位不再静默回落",
    "slow-stream":           "§3.2 沉默的流式上游：新版插心跳，v1 纯直通",
    "title-gen":             "§5.5.4 标题生成降到最省档",
    "chain-reject":          "§3.11 两个整流器在同一请求里连续触发",
    "loop-reject":           "§3.11 防循环上限：最多整流 2 次",
    "budget-reject":         "§3.11.2 budget 下限 → 抬到 32000 重试",
    "sig-reject":            "§3.11.3 thinking 块签名 → 剥掉后重试",
    "effort-reject-1":       "§3.11.1 上游拒收 output_config → 去掉重试",
    "effort-reject-2":       "§3.11.1 能力缓存命中，effort 直接不发",
}

def load(label):
    out = {}
    for line in (eq / f"cap-{label}.jsonl").read_text().splitlines():
        rec = json.loads(line)
        tag = ((rec.get("body") or {}).get("metadata") or {}).get("user_id", "(untagged)")
        out.setdefault(tag, []).append(rec)
    return out

old, new = load("old"), load("new")
fails = 0

# ---- 表外用例：逐字节等价 ----
same = sorted((set(old) | set(new)) - set(DIVERGE))
for tag in same:
    o, n = old.get(tag, []), new.get(tag, [])
    if o == n:
        print(f"✓ {tag}: 与 v1 一致（{len(n)} 次转发）")
    else:
        print(f"✗ {tag}: 与 v1 不一致")
        print(f"    old={json.dumps(o, ensure_ascii=False, sort_keys=True)}")
        print(f"    new={json.dumps(n, ensure_ascii=False, sort_keys=True)}")
        fails += 1

def check(tag, cond, what):
    global fails
    print(("✓ " if cond else "✗ ") + f"{tag}: {what}")
    if not cond:
        got = json.dumps(new.get(tag), ensure_ascii=False, sort_keys=True)
        print(f"    实际: {got}")
        fails += 1

# ---- DIVERGE 用例：断言新行为 ----
print(f"-- DIVERGE（{len(DIVERGE)} 项有意改动） --")

# §3.10 服务商 te=off + 请求自带 effort → 原样透传，thinking 不写 disabled
t = "slot1-off-with-effort"
b = (new.get(t) or [{}])[0].get("body", {})
check(t, len(new.get(t, [])) == 1
         and b.get("output_config") == {"effort": "low"}
         and "thinking" not in b,
      "effort 透传、未写 thinking:disabled（v1 会踩掉）")

# §3.10 服务商 te=high + 请求自带 effort/adaptive → 两者都不动
t = "slot2-passthrough"
b = (new.get(t) or [{}])[0].get("body", {})
check(t, len(new.get(t, [])) == 1
         and b.get("output_config") == {"effort": "low"}
         and b.get("thinking") == {"type": "adaptive"},
      "effort 与 adaptive 思考均原样透传（v1 会覆盖成 enabled+8192/high）")

# §3.3 未映射槽位 → 直接 400，一次上游都不打（v1 会回落到 real-a 照发）
t = "unmapped"
check(t, not new.get(t) and len(old.get(t, [])) == 1,
      "未转发到上游（v1 静默回落到第一个模型）")

# §3.10 兜底注入：与桌面端同款形态（effort + adaptive），不再写死 budget_tokens。
# v1 写的是 {"type":"enabled","budget_tokens":8192} —— 一个没有出处的裸字面量。
for t, effort in (("slot2-high", "high"), ("slot3-max-headers", "max")):
    recs = new.get(t, [])
    b = (recs or [{}])[0].get("body", {})
    ok = (len(recs) == 1
          and b.get("output_config") == {"effort": effort}
          and b.get("thinking") == {"type": "adaptive"}
          and old.get(t, [{}])[0].get("body", {}).get("thinking", {}).get("budget_tokens") == 8192)
    check(t, ok, f"effort={effort} + adaptive；v1 对照写的是 enabled+8192")
    # 头透传仍须与 v1 一致（这条没变）
    if t == "slot3-max-headers" and recs:
        same_headers = recs[0]["headers"] == old.get(t, [{}])[0].get("headers")
        check(t + "/headers", same_headers, "请求头与 v1 逐字节一致")

# §5.5.4 标题生成：降到 effort=low + thinking disabled（v1 原样转发）
t = "title-gen"
recs = new.get(t, [])
ok = len(recs) == 1
if ok:
    b = recs[0]["body"]
    ok = (b.get("output_config") == {"effort": "low"}
          and b.get("thinking") == {"type": "disabled"}
          and old.get(t, [{}])[0].get("body", {}).get("output_config") is None)
check(t, ok, "已降到最省档，且 v1 对照原样转发")

# §3.2 心跳不改请求体，只影响响应流：两次转发（流式 + 非流式）都应原样到达上游
t = "slow-stream"
recs = new.get(t, [])
check(t, len(recs) == 2 and all("metadata" in r.get("body", {}) for r in recs),
      "请求体原样转发（心跳只加在响应侧）")

# §3.11 组合路径：两个整流器在同一请求里先后触发
t = "chain-reject"
recs = new.get(t, [])
ok = len(recs) == 3
if ok:
    b1, b2, b3 = (r["body"] for r in recs)
    ok = (b1["thinking"]["budget_tokens"] == 100                      # 原样
          and b2["thinking"]["budget_tokens"] == 32000                # 修了 budget
          and any(blk.get("type") == "thinking"
                  for m in b2["messages"] if isinstance(m.get("content"), list)
                  for blk in m["content"])                            # 此时块还在
          and all(blk.get("type") != "thinking"
                  for m in b3["messages"] if isinstance(m.get("content"), list)
                  for blk in m["content"]))                           # 第三次才剥块
check(t, ok, "三次转发：budget → thinking 块，两个整流器依次生效")

# §3.11 防循环：最多整流 2 次 → 至多 3 次转发
t = "loop-reject"
recs = new.get(t, [])
check(t, len(recs) == 3, f"怎么改都拒时只转发 3 次（实得 {len(recs)}）")

# §3.11.2 budget 下限：抬到 32000，且 max_tokens 一并提到 64000（budget 必须更小）
t = "budget-reject"
recs = new.get(t, [])
ok = len(recs) == 2
if ok:
    first, second = recs[0]["body"], recs[1]["body"]
    ok = (first["thinking"]["budget_tokens"] == 100
          and second["thinking"] == {"type": "enabled", "budget_tokens": 32000}
          and second["max_tokens"] == 64000)
check(t, ok, "两次转发：budget 100 被拒 → 32000 + max_tokens 提到 64000")

# §3.11.3 thinking 块签名：剥掉历史块与签名后重试
t = "sig-reject"
recs = new.get(t, [])
ok = len(recs) == 2
if ok:
    kept = recs[1]["body"]["messages"]
    ok = (len(recs[0]["body"]["messages"]) == 2
          and all(b.get("type") != "thinking"
                  for m in kept if isinstance(m.get("content"), list)
                  for b in m["content"])
          and all("signature" not in b
                  for m in kept if isinstance(m.get("content"), list)
                  for b in m["content"]))
check(t, ok, "两次转发：第二次已剥掉 thinking 块与 signature")

# §3.11.1 首发被拒 → 去掉 output_config 重试一次
t = "effort-reject-1"
recs = new.get(t, [])
check(t, len(recs) == 2
         and "output_config" in recs[0].get("body", {})
         and "output_config" not in recs[1].get("body", {}),
      "两次转发：第一次带 effort 被拒，第二次去掉后成功")

# §3.11.1 副作用：能力缓存命中，后续请求直接不发 effort
t = "effort-reject-2"
recs = new.get(t, [])
check(t, len(recs) == 1 and "output_config" not in recs[0].get("body", {}),
      "只有一次转发且不带 effort（能力缓存生效）")

sys.exit(1 if fails else 0)
PY
then :; else fail=1; fi

echo "=== 新行为响应断言（2.1-A） ==="
if [ "$(cat "$EQ/out-new/unmapped.status")" = "400" ] &&
   python3 -c "
import json,sys
b = json.load(open('$EQ/out-new/unmapped.body'))
m = b['error']['message']
sys.exit(0 if b['type']=='error' and b['error']['type']=='invalid_request_error'
             and m.startswith('ModelLink: 模型槽位 claude-nonexistent-9 未映射') else 1)"; then
  echo "✓ 未映射槽位返回 400 + Anthropic 错误体（槽位名已剥 [1m]）"
else
  echo "✗ 未映射槽位响应异常: $(cat "$EQ/out-new/unmapped.status") $(cat "$EQ/out-new/unmapped.body")"; fail=1
fi
if [ "$(cat "$EQ/out-old/unmapped.status")" = "200" ]; then
  echo "✓ 对照：v1 静默回落并返回 200"
else
  echo "✗ 对照失败：v1 的 unmapped.status = $(cat "$EQ/out-old/unmapped.status")"; fail=1
fi
if [ "$(cat "$EQ/out-new/rectify.status")" = "200" ] && grep -q '"text": *"ok"\|"text":"ok"' "$EQ/out-new/rectify.body"; then
  echo "✓ 整流后对下游返回 200（桌面端看不到上游那个 400，也就不会锁会话）"
else
  echo "✗ 整流后响应异常: $(cat "$EQ/out-new/rectify.status") $(cat "$EQ/out-new/rectify.body")"; fail=1
fi
if [ "$(cat "$EQ/out-new/chain.status")" = "200" ]; then
  echo "✓ 连环整流后对下游返回 200"
else
  echo "✗ 连环整流失败: $(cat "$EQ/out-new/chain.status")"; fail=1
fi
if [ "$(cat "$EQ/out-new/loop.status")" = "400" ] && grep -q "尝试过的自动修复" "$EQ/out-new/loop.body"; then
  echo "✓ 整流救不回来时原样返回最初的错误，并附上尝试过的修复"
else
  echo "✗ 防循环路径的响应不对: $(cat "$EQ/out-new/loop.status") $(cat "$EQ/out-new/loop.body")"; fail=1
fi
for f in budget sig; do
  if [ "$(cat "$EQ/out-new/$f.status")" = "200" ]; then
    echo "✓ $f 整流后对下游返回 200"
  else
    echo "✗ $f 整流后响应异常: $(cat "$EQ/out-new/$f.status") $(cat "$EQ/out-new/$f.body")"; fail=1
  fi
  if [ "$(cat "$EQ/out-old/$f.status")" = "400" ]; then
    echo "✓ 对照：v1 把 $f 的 400 原样吐给桌面端"
  else
    echo "✗ 对照失败：v1 的 $f.status = $(cat "$EQ/out-old/$f.status")"; fail=1
  fi
done
if [ "$(cat "$EQ/out-old/rectify.status")" = "400" ]; then
  echo "✓ 对照：v1 把上游 400 原样吐给桌面端"
else
  echo "✗ 对照失败：v1 的 rectify.status = $(cat "$EQ/out-old/rectify.status")"; fail=1
fi
echo "=== §3.2 SSE 心跳（上游沉默 20s，心跳固定 15s） ==="
if python3 - "$EQ" <<'PY'
import sys, pathlib
eq = pathlib.Path(sys.argv[1])
fails = 0

new_body = (eq / "out-new" / "heartbeat.body").read_text()
old_body = (eq / "out-old" / "heartbeat.body").read_text()
pings = new_body.count(": ping")
# 20s 沉默 / 15s 间隔 → 至少 1 次
if pings >= 1:
    print(f"✓ 流式沉默期间下游收到 {pings} 次 `: ping`")
else:
    print(f"✗ 心跳次数不足: {pings} 次，body={new_body!r}"); fails += 1
if "message_stop" not in new_body:
    print("✗ 上游真数据没能透传到下游"); fails += 1
else:
    print("✓ 心跳之后上游真事件照常透传")
if ": ping" in old_body:
    print("✗ 对照失败：v1 竟然也发了心跳"); fails += 1
else:
    print("✓ 对照：v1 纯直通，沉默期间一个字节都没有")

# 非流式响应绝不能被插入注释行 —— 那会让 JSON 解析失败
nostream = (eq / "out-new" / "nostream.body").read_text()
if ": ping" in nostream:
    print("✗ 非流式响应被插入了心跳，JSON 已污染"); fails += 1
else:
    print("✓ 非流式响应未插心跳")
sys.exit(1 if fails else 0)
PY
then :; else fail=1; fi

echo "=== /v1/models（2.1-B 槽位池已换，比对映射顺序而非槽位名） ==="
if python3 - "$EQ" <<'PY'
import json, sys, pathlib
eq = pathlib.Path(sys.argv[1])
POOL = ["claude-opus-5", "claude-sonnet-5", "claude-opus-4-8", "claude-opus-4-7",
        "claude-opus-4-6", "claude-sonnet-4-6", "claude-sonnet-4-5", "claude-haiku-4-5"]
o = json.load(open(eq / "out-old" / "models.json"))["data"]
n = json.load(open(eq / "out-new" / "models.json"))["data"]
fails = 0
# v1 封顶 8 个模型、新版 20（§3.6 有意扩容）→ 只比对 v1 能表达的那一段
o_names = [m["display_name"] for m in o]
n_names = [m["display_name"] for m in n]
if n_names[: len(o_names)] != o_names:
    print("✗ 前 8 个槽位的映射顺序与 v1 不一致")
    print(f"    old={o_names}\n    new={n_names[: len(o_names)]}")
    fails += 1
elif len(n_names) <= len(o_names):
    print(f"✗ 新版没有超出 v1 的 8 槽位上限（{len(n_names)} 条）"); fails += 1
else:
    print(f"✓ 前 {len(o_names)} 条映射与 v1 一致，且已扩到 {len(n_names)} 条")
for m in n:
    slot = m["id"].removesuffix("[1m]")
    if slot not in POOL and not slot.startswith("claude-ml-"):
        print(f"✗ 槽位 {slot} 不在 2.1 槽位池里"); fails += 1
if not fails:
    print("✓ 槽位全部来自 2.1 新池子")

# §3.6 溢出层：配置里有 22 个模型，应当正好取前 20 个，第 9 个起走 claude-ml-{n}
slots = [m["id"] for m in n if not m["id"].endswith("[1m]")]
if len(slots) != 20:
    print(f"✗ 应封顶 20 个槽位，实得 {len(slots)}"); fails += 1
elif slots[:8] != POOL or slots[8] != "claude-ml-1" or slots[19] != "claude-ml-12":
    print(f"✗ 溢出层分配不对: {slots}"); fails += 1
else:
    print("✓ 封顶 20 槽位，第 9 个起走 claude-ml-{n} 溢出层")
sys.exit(1 if fails else 0)
PY
then :; else fail=1; fi
echo "=== DIFF 响应（状态码/透传体/404/502 话术） ==="
for f in r1.status r1.body notfound.status notfound.body nomodel.status nomodel.body; do
  if diff "$EQ/out-old/$f" "$EQ/out-new/$f" >/dev/null; then echo "✓ $f"; else echo "✗ $f"; fail=1; fi
done
echo "=== 网关写入对比（Claude-3p，红线 #3） ==="
if python3 - "$EQ" <<'PY'
import json, sys, pathlib
eq = pathlib.Path(sys.argv[1])
base = "Library/Application Support/Claude-3p"
fails = 0

# 新版比 v1 多写的键 —— 剔除后其余必须逐字节一致
# labelOverride: 2026-07-14 拍板例外；后两个: 2.1-A §3.4
ADDED_KEYS = {"chatTabEnabled": True, "disableDeploymentModeChooser": True,
              "inferenceStreamIdleTimeoutSec": 1800,
              # §5.5.5：槽位名都是完整 ID，app 本就会跳过发现流程，显式关掉免得白跑往返
              "modelDiscoveryEnabled": False}
# 2.1-B §3.1：费率两键只在「应用」时写（那时才有模型与费率），
# 启动自动配置阶段有意不碰 —— 否则每次重启都会把用户刚应用好的费率表清掉。
PRICING_KEYS_MUST_BE_ABSENT = ["inferenceModelPricingEnabled", "inferenceModelPricing"]
POOL = ["claude-opus-5", "claude-sonnet-5", "claude-opus-4-8", "claude-opus-4-7",
        "claude-opus-4-6", "claude-sonnet-4-6", "claude-sonnet-4-5", "claude-haiku-4-5"]

cases = [
    ("configLibrary/a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4.json", True),
    ("configLibrary/_meta.json", False),
    ("claude_desktop_config.json", False),
]
for rel, is_gateway in cases:
    o = json.load(open(eq / "home-old" / base / rel))
    n = json.load(open(eq / "home-new" / base / rel))
    if is_gateway:
        for m in n.get("inferenceModels", []):
            if m.pop("labelOverride", None) is None:
                print(f"✗ {rel}: 新版条目缺 labelOverride"); fails += 1
        for k, want in ADDED_KEYS.items():
            if n.pop(k, None) != want:
                print(f"✗ {rel}: 缺 {k}={want}（§3.4）"); fails += 1
            elif k in o:
                print(f"✗ {rel}: v1 竟然也写了 {k}?"); fails += 1
        # 2.1-B 槽位池：inferenceModels 的 name 与 v1 必然不同，按顺序对齐后比对
        for side in (o, n):
            side.setdefault("inferenceModels", [])
        if len(o["inferenceModels"]) == len(n["inferenceModels"]):
            for m in n["inferenceModels"]:
                if m.get("name") not in POOL and not str(m.get("name")).startswith("claude-ml-"):
                    print(f"✗ {rel}: 槽位 {m.get('name')} 不在 2.1 槽位池里"); fails += 1
            o["inferenceModels"] = n["inferenceModels"] = "<按顺序对齐后忽略槽位名>"
        for k in PRICING_KEYS_MUST_BE_ABSENT:
            if k in n:
                print(f"✗ {rel}: 启动自动配置不该写 {k}（会清掉已应用的费率表）"); fails += 1
        # v1 从不写这两个键 → 用户装完 Chat 页是关的，这正是 §3.4 要修的
    if o == n:
        print(f"✓ {rel}" + ("（剔除新增键后与 v1 一致）" if is_gateway else ""))
    else:
        print(f"✗ {rel} 结构不一致")
        print(f"    old={json.dumps(o, ensure_ascii=False, sort_keys=True)}")
        print(f"    new={json.dumps(n, ensure_ascii=False, sort_keys=True)}")
        fails += 1
sys.exit(1 if fails else 0)
PY
then :; else fail=1; fi
echo "=== LaunchAgent 迁移（红线 #5） ==="
if [ -f "$EQ/home-new/Library/LaunchAgents/com.modellink.plist" ]; then echo "✗ 旧 plist 未删除"; fail=1; else echo "✓ 旧 com.modellink.plist 已删除"; fi
ls "$EQ/home-new/Library/LaunchAgents/" 2>/dev/null | sed 's/^/  新注册: /'

echo
[ "$fail" -eq 0 ] && echo "ALL GREEN ✓ ($EQ)" || { echo "FAILURES ✗ ($EQ)"; exit 1; }
