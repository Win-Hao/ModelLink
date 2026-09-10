#!/usr/bin/env python3
"""
服务商能力探针 —— 建预设库用，也是「测试连接」应该做的事的原型。

逐项探测一个 Anthropic 兼容端点到底支持到什么程度，输出可直接填进
src/lib/presets.ts 的字段。

用法：
    python3 probe-provider.py <base_url> <api_key> <model_id>

示例：
    python3 probe-provider.py https://api.kimi.com/coding sk-xxx k3
    python3 probe-provider.py https://open.bigmodel.cn/api/anthropic xxx glm-5.1
"""
import json, sys, time, urllib.request, urllib.error

TIMEOUT = 120


def call(base, key, body, headers=None, path="/v1/messages", method="POST"):
    h = {"Authorization": "Bearer " + key,
         "anthropic-version": "2023-06-01",
         "content-type": "application/json"}
    if headers:
        h.update(headers)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base.rstrip("/") + path, data=data, headers=h, method=method)
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            return r.status, json.loads(r.read().decode()), round(time.time() - t0, 2)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode()), round(time.time() - t0, 2)
        except Exception:
            return e.code, {"_raw": "<unparsable>"}, round(time.time() - t0, 2)
    except Exception as e:
        return None, {"_err": f"{type(e).__name__}: {e}"}, round(time.time() - t0, 2)


def ok(label, cond, extra=""):
    print(f"  [{'✓' if cond else '✗'}] {label:<44} {extra}")
    return cond


def main(base, key, model):
    print(f"\n探测 {base}   model={model}\n" + "=" * 78)
    out = {}

    # ---------- 1. 基础 Messages API ----------
    print("\n1. 基础 Messages API（必须，不通则接不上 Claude Desktop）")
    st, d, sec = call(base, key, {"model": model, "max_tokens": 1,
                                  "messages": [{"role": "user", "content": "hi"}]})
    out["messages"] = st == 200
    ok("POST /v1/messages", st == 200, f"HTTP {st}  {sec}s  echo_model={d.get('model')!r}")
    if st != 200:
        print(f"      {json.dumps(d, ensure_ascii=False)[:200]}")
        return out

    # ---------- 2. 模型发现 ----------
    print("\n2. 模型发现（可选）")
    st, d, sec = call(base, key, None, path="/v1/models", method="GET")
    out["models_endpoint"] = st == 200
    ids = [m.get("id") for m in (d.get("data") or [])] if st == 200 else []
    ok("GET /v1/models", st == 200, f"HTTP {st}  {ids[:6]}")
    if ids:
        tiers = {m.get("id"): m.get("anthropic_family_tier")
                 for m in d["data"] if m.get("anthropic_family_tier")}
        ok("返回 anthropic_family_tier", bool(tiers), str(tiers) if tiers else "（无，模型名必须自带 claude 字样才进得了选择器）")
        eff = {m.get("id"): (m.get("think_efforts") or {}).get("valid_efforts")
               for m in d["data"] if m.get("think_efforts")}
        if eff:
            print(f"      上游自述档位: {eff}")
            out["upstream_efforts"] = eff

    # ---------- 3. 模型名校验严格度 ----------
    print("\n3. 模型名校验（决定能不能用 Claude 槽位名伪装）")
    st, d, _ = call(base, key, {"model": "zzz-nonexistent-xyz", "max_tokens": 1,
                                "messages": [{"role": "user", "content": "hi"}]})
    strict = st != 200
    out["validates_model_name"] = strict
    ok("拒绝不存在的模型名", strict,
       f"HTTP {st}" + ("" if strict else "  ⚠ 会静默回落到默认模型，用户不知道自己在用什么"))

    st, d, _ = call(base, key, {"model": "claude-opus-5", "max_tokens": 1,
                                "messages": [{"role": "user", "content": "hi"}]})
    out["accepts_claude_slot"] = st == 200
    ok("接受 claude-opus-5 这类槽位名", st == 200,
       f"HTTP {st}" + ("" if st == 200 else f"  → 必须靠代理改写 model 字段"))

    # ---------- 4. 推理强度 output_config.effort ----------
    print("\n4. 推理强度 output_config.effort（Claude Desktop 选择器发的就是这个）")
    accepted = {}
    for lv in ["low", "medium", "high", "xhigh", "max"]:
        st, d, sec = call(base, key,
                          {"model": model, "max_tokens": 1, "output_config": {"effort": lv},
                           "messages": [{"role": "user", "content": "."}]},
                          headers={"anthropic-beta": "effort-2025-11-24"})
        u = d.get("usage", {}) if st == 200 else {}
        th = (u.get("output_tokens_details") or {}).get("thinking_tokens")
        accepted[lv] = st == 200
        print(f"      effort={lv:<7} HTTP {st}  thinking={th}  {sec}s"
              + ("" if st == 200 else f"  {json.dumps(d, ensure_ascii=False)[:110]}"))
    out["effort_accepted"] = accepted
    all_ok = all(accepted.values())
    ok("五档全部接受", all_ok,
       "→ 桌面端选择器可直接用，ModelLink 透传即可" if all_ok
       else "→ 需要代理层翻译成该服务商自己的思考参数")

    # ---------- 5. 原生 thinking 字段 ----------
    print("\n5. Anthropic 原生 thinking 字段（effort 不支持时的替代路径）")
    for th_body in [{"type": "adaptive"},
                    {"type": "enabled", "budget_tokens": 4096},
                    {"type": "disabled"}]:
        st, d, _ = call(base, key,
                        {"model": model, "max_tokens": 1, "thinking": th_body,
                         "messages": [{"role": "user", "content": "."}]})
        u = d.get("usage", {}) if st == 200 else {}
        t = (u.get("output_tokens_details") or {}).get("thinking_tokens")
        print(f"      thinking={json.dumps(th_body):<44} HTTP {st}  thinking_tokens={t}")
        out.setdefault("thinking_variants", {})[th_body["type"]] = st == 200

    # ---------- 6. Prompt caching 透传 ----------
    print("\n6. Prompt caching 透传（不支持则每轮全量重算，成本和延迟都翻倍）")
    big = "背景资料。" * 200
    body = {"model": model, "max_tokens": 1,
            "system": [{"type": "text", "text": big, "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": "."}]}
    st1, d1, _ = call(base, key, body)
    time.sleep(2)
    st2, d2, _ = call(base, key, body)
    u1, u2 = d1.get("usage", {}), d2.get("usage", {})
    cw = u1.get("cache_creation_input_tokens")
    cr = u2.get("cache_read_input_tokens")
    out["prompt_caching"] = bool(cr)
    ok("cache_control 生效", bool(cr), f"首次 write={cw}  二次 read={cr}")

    # ---------- 7. 1M 上下文 beta ----------
    print("\n7. 1M 上下文 beta 头（supports1m 打开后引擎会发这个）")
    st, d, _ = call(base, key, {"model": model, "max_tokens": 1,
                                "messages": [{"role": "user", "content": "hi"}]},
                    headers={"anthropic-beta": "context-1m-2025-08-07"})
    out["accepts_1m_beta"] = st == 200
    ok("接受 context-1m-2025-08-07", st == 200, f"HTTP {st}")

    # ---------- 汇总 ----------
    print("\n" + "=" * 78)
    print("可填进 presets.ts 的结论：")
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


if __name__ == "__main__":
    if len(sys.argv) != 4:
        print(__doc__)
        sys.exit(1)
    main(sys.argv[1], sys.argv[2], sys.argv[3])
