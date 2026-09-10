//! 服务商能力探针（§5.6）：把「测试连接」从「通不通」升级成「支持到什么程度」。
//!
//! 现状只发一个 1-token 请求判连通。第三方 Anthropic 兼容端点的兼容度参差不齐，
//! 用户接完根本不知道这家支不支持推理强度、会不会静默回落模型、透不透传缓存 ——
//! 而这三件事直接决定 ModelLink 的哪些能力在这家上面能用。
//!
//! 逐项探测的清单与 `scripts/probe-provider.py` 一致（那是本模块的原型）。

use serde::Serialize;

/// 单项探测结论。`None` = 没测（前置项没通过就不往下测了）。
#[derive(Serialize, Default, Clone, Debug)]
pub struct ProbeReport {
    /// 基础 `POST /v1/messages`，不通则接不上 Claude Desktop。
    pub ok: bool,
    /// 不通时的原因（连通时为空）。
    pub message: String,
    /// `GET /v1/models` 可用。
    pub models_endpoint: bool,
    /// 上游自述的推理强度档位（Kimi 会返回 `think_efforts.valid_efforts`）。
    pub upstream_efforts: Vec<String>,
    /// 会拒绝不存在的模型名 —— false 表示会**静默回落**到默认模型。
    pub validates_model_name: bool,
    /// 接受 `claude-opus-5` 这类槽位名（不接受就必须靠代理改写 model 字段）。
    pub accepts_claude_slot: bool,
    /// `output_config.effort` 五档各自的接受情况。
    ///
    /// ⚠️ 「接受」= 没被拒，**不等于真的生效**。部分服务商在网关侧把档位映射到自己的
    /// 档位（Kimi 官方只有 low/high/max，medium→high、xhigh→max），也有端点会收下
    /// 字段后原样忽略。要证明生效只能看思考量，见 `effort_thinking`。
    pub effort_accepted: Vec<(String, bool)>,
    /// 各档位实测的思考 token 数（`usage.output_tokens_details.thinking_tokens`）。
    /// 探测请求刻意很小，多数情况下全是 0 —— 只有当各档确实拉开差距时才是「已生效」的实证。
    pub effort_thinking: Vec<(String, u64)>,
    /// 原生 thinking 三形态的接受情况。
    pub thinking_variants: Vec<(String, bool)>,
    /// 本次探测**观察到**了缓存命中。
    ///
    /// ⚠️ false 不等于「这家不支持缓存」：实测 Kimi Code 对合成请求即便 12k token
    /// 也不建缓存，但真实 Chat 会话里能看到 cache_read_input_tokens。
    pub prompt_caching: bool,
    /// 接受 `context-1m-2025-08-07` beta 头。
    pub accepts_1m_beta: bool,
    pub elapsed_ms: u64,
}

/// 五档推理强度，顺序与 Claude Desktop 选择器一致（xhigh 在 UI 上显示为 Extra）。
pub const EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// 从 `/v1/models` 响应里挖出上游自述的档位。
/// Kimi 的 k3 会返回 `think_efforts: { valid_efforts: [...] }`；多数服务商不返回。
pub fn parse_upstream_efforts(payload: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(items) = payload.get("data").and_then(|d| d.as_array()) else {
        return out;
    };
    for m in items {
        let Some(list) = m
            .get("think_efforts")
            .and_then(|t| t.get("valid_efforts"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for v in list {
            if let Some(s) = v.as_str() {
                if !out.iter().any(|x| x == s) {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

/// 响应里的思考 token 数（Kimi 等按 Anthropic 格式的端点放在这里）。
pub fn thinking_tokens(body: &serde_json::Value) -> u64 {
    body.get("usage")
        .and_then(|u| u.get("output_tokens_details"))
        .and_then(|d| d.get("thinking_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// 各档位的思考量是否确实拉开了差距 —— 这是「effort 真的生效」的唯一实证。
/// 全 0 或全相同都说明看不出来（探测请求太小），**不代表没生效**。
pub fn effort_visibly_differs(samples: &[(String, u64)]) -> bool {
    let nonzero: Vec<u64> = samples.iter().map(|(_, v)| *v).filter(|v| *v > 0).collect();
    nonzero.len() >= 2 && nonzero.iter().min() != nonzero.iter().max()
}

/// 上游是否真的把缓存读回来了。两次同样的大 system 请求，第二次应出现
/// `cache_read_input_tokens > 0`。
pub fn cache_hit(second: &serde_json::Value) -> bool {
    second
        .get("usage")
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        > 0
}

/// 探测结论里最值得让用户当场知道的几句话。
pub fn headlines(r: &ProbeReport) -> Vec<String> {
    let mut out = Vec::new();
    if !r.ok {
        return out;
    }
    if !r.validates_model_name {
        out.push(
            "⚠ 这家会**静默回落**：请求一个不存在的模型也返回 200，直接给默认模型。\
             ModelLink 已经在自己这层拦住未映射的槽位，但上游这层拦不住。"
                .to_string(),
        );
    }
    let accepted: Vec<&str> = r
        .effort_accepted
        .iter()
        .filter(|(_, ok)| *ok)
        .map(|(k, _)| k.as_str())
        .collect();
    if accepted.len() == EFFORT_LEVELS.len() {
        if effort_visibly_differs(&r.effort_thinking) {
            out.push("✓ 五个档位均被接受，且各档思考量确有差异 —— 已确认真实生效。".to_string());
        } else {
            out.push(
                "✓ 五个档位均未被拒。⚠ 但没能确认是否真的生效：探测请求很小，看不出思考量差异；                 而且部分服务商会在网关侧把档位映射到自己的档（如 Kimi 官方只有 low/high/max）。"
                    .to_string(),
            );
        }
    } else if accepted.is_empty() {
        out.push("✗ 不接受 output_config.effort，代理会在首次被拒后自动去掉并记住。".to_string());
    } else {
        out.push(format!("△ 只接受部分推理强度档位：{}。", accepted.join(" / ")));
    }
    if !r.upstream_efforts.is_empty() {
        out.push(format!("上游自述档位：{}。", r.upstream_efforts.join(" / ")));
    }
    if !r.prompt_caching {
        // 只报观察到的事实，不替上游下结论。
        // 实测 Kimi Code：合成请求即便 12k token 也不建缓存（cache_creation 恒为 0），
        // 但真实 Chat 会话里能看到 cache_read_input_tokens: 9728 —— 也就是说
        // 这一项探不到不等于这家不支持缓存，可能只是不吃这种探测形态。
        out.push(
            "△ 本次探测未观察到缓存命中（合成请求可能触发不了上游的缓存条件，             真实多轮会话里未必如此）。"
                .to_string(),
        );
    }
    if !r.accepts_1m_beta {
        out.push("⚠ 不接受 1M 上下文 beta 头，模型上的「1M」开关对这家无效。".to_string());
    }
    if !r.accepts_claude_slot {
        out.push("这家不认 claude-* 槽位名 —— 不影响使用，代理本来就会改写成真实模型名。".to_string());
    }
    out
}

/// 单次探测请求：返回 (HTTP 状态, 响应体)。网络层出错时状态为 None。
async fn call(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    body: Option<serde_json::Value>,
    beta: Option<&str>,
    path: &str,
) -> (Option<u16>, serde_json::Value) {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let mut req = match &body {
        Some(_) => client.post(&url),
        None => client.get(&url),
    };
    req = req
        .header("authorization", format!("Bearer {}", key))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(60));
    if let Some(b) = beta {
        req = req.header("anthropic-beta", b);
    }
    if let Some(b) = &body {
        req = req.body(serde_json::to_vec(b).unwrap_or_default());
    }
    match req.send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let raw = r.bytes().await.unwrap_or_default();
            let v = serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
            (Some(status), v)
        }
        Err(e) => (None, serde_json::json!({ "_err": e.to_string() })),
    }
}

fn ping(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "."}]
    })
}

/// 跑完整套探测（§5.6）。全部请求都是 max_tokens=1，代价极小；
/// 独立的几组并发发出去，免得用户对着按钮等太久。
pub async fn run(client: &reqwest::Client, base: &str, key: &str, model: &str) -> ProbeReport {
    let t0 = std::time::Instant::now();
    let mut r = ProbeReport::default();

    // 1. 基础 Messages API —— 不通就没有往下测的意义
    let (st, body) = call(client, base, key, Some(ping(model)), None, "/v1/messages").await;
    r.ok = st == Some(200);
    if !r.ok {
        r.message = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(String::from)
            .or_else(|| body.get("_err").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| match st {
                Some(code) => format!("HTTP {code}"),
                None => "无法连接".to_string(),
            });
        r.elapsed_ms = t0.elapsed().as_millis() as u64;
        return r;
    }

    // 2/3/7 并发：模型发现、模型名校验、槽位名接受度、1M beta 头
    let models_f = call(client, base, key, None, None, "/v1/models");
    let bogus_f = call(
        client,
        base,
        key,
        Some(ping("zzz-nonexistent-xyz")),
        None,
        "/v1/messages",
    );
    let slot_f = call(client, base, key, Some(ping("claude-opus-5")), None, "/v1/messages");
    let beta_f = call(
        client,
        base,
        key,
        Some(ping(model)),
        Some("context-1m-2025-08-07"),
        "/v1/messages",
    );
    let (models, bogus, slot, beta) = tokio::join!(models_f, bogus_f, slot_f, beta_f);

    r.models_endpoint = models.0 == Some(200);
    if r.models_endpoint {
        r.upstream_efforts = parse_upstream_efforts(&models.1);
    }
    // 拒绝不存在的模型名 = 不会静默回落
    r.validates_model_name = bogus.0 != Some(200);
    r.accepts_claude_slot = slot.0 == Some(200);
    r.accepts_1m_beta = beta.0 == Some(200);

    // 4. 五档推理强度（并发）
    let effort_reqs: Vec<_> = EFFORT_LEVELS
        .iter()
        .map(|lv| {
            let mut b = ping(model);
            b["output_config"] = serde_json::json!({ "effort": lv });
            (b, Some("effort-2025-11-24"))
        })
        .collect();
    for (lv, (st, body)) in EFFORT_LEVELS
        .iter()
        .zip(call_all(client, base, key, effort_reqs).await)
    {
        r.effort_accepted.push((lv.to_string(), st == Some(200)));
        if st == Some(200) {
            r.effort_thinking.push((lv.to_string(), thinking_tokens(&body)));
        }
    }

    // 5. 原生 thinking 三形态（并发）
    let variants = [
        ("adaptive", serde_json::json!({"type": "adaptive"})),
        ("enabled", serde_json::json!({"type": "enabled", "budget_tokens": 4096})),
        ("disabled", serde_json::json!({"type": "disabled"})),
    ];
    let think_reqs: Vec<_> = variants
        .iter()
        .map(|(_, v)| {
            let mut b = ping(model);
            // enabled + budget 要求 max_tokens > budget
            b["max_tokens"] = serde_json::json!(8192);
            b["thinking"] = v.clone();
            (b, None)
        })
        .collect();
    for ((name, _), (st, _)) in variants
        .iter()
        .zip(call_all(client, base, key, think_reqs).await)
    {
        r.thinking_variants.push((name.to_string(), st == Some(200)));
    }

    // 6. Prompt caching 透传 —— 必须串行：先写后读
    //
    // ⚠️ system 块必须足够大：缓存有最小 token 门槛（Anthropic 侧是 1024，
    // Haiku 类小模型 2048），低于门槛上游根本不会建缓存，探测就会得到假阴性。
    // 原型脚本用的 ×200（约 700 token）实测就会误报 Kimi 不支持缓存。
    let big = "背景资料。".repeat(800);
    let cache_body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "system": [{"type": "text", "text": big, "cache_control": {"type": "ephemeral"}}],
        "messages": [{"role": "user", "content": "."}]
    });
    let _ = call(client, base, key, Some(cache_body.clone()), None, "/v1/messages").await;
    // 缓存建立是异步的，紧跟着读多半 miss —— 等一下再问第二遍
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let (_, second) = call(client, base, key, Some(cache_body), None, "/v1/messages").await;
    r.prompt_caching = cache_hit(&second);

    r.elapsed_ms = t0.elapsed().as_millis() as u64;
    r
}

/// 并发跑一组探测请求。`tokio::join!` 要求编译期已知个数，这里个数随档位表变，
/// 所以走 spawn（reqwest::Client 内部是 Arc，clone 很便宜）。
async fn call_all(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    reqs: Vec<(serde_json::Value, Option<&'static str>)>,
) -> Vec<(Option<u16>, serde_json::Value)> {
    let handles: Vec<_> = reqs
        .into_iter()
        .map(|(body, beta)| {
            let (c, b, k) = (client.clone(), base.to_string(), key.to_string());
            tokio::spawn(async move {
                call(&c, &b, &k, Some(body), beta, "/v1/messages").await
            })
        })
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap_or((None, serde_json::Value::Null)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_efforts_are_collected_and_deduped() {
        let payload = serde_json::json!({"data": [
            {"id": "k3", "think_efforts": {"valid_efforts": ["low", "high", "max"]}},
            {"id": "k3-256k", "think_efforts": {"valid_efforts": ["low", "high"]}},
            {"id": "other"}
        ]});
        assert_eq!(parse_upstream_efforts(&payload), vec!["low", "high", "max"]);
        assert!(parse_upstream_efforts(&serde_json::json!({})).is_empty());
        assert!(parse_upstream_efforts(&serde_json::json!({"data": []})).is_empty());
    }

    #[test]
    fn cache_hit_needs_a_nonzero_read() {
        assert!(cache_hit(&serde_json::json!({"usage": {"cache_read_input_tokens": 9728}})));
        assert!(!cache_hit(&serde_json::json!({"usage": {"cache_read_input_tokens": 0}})));
        assert!(!cache_hit(&serde_json::json!({"usage": {}})));
        assert!(!cache_hit(&serde_json::json!({})));
    }

    #[test]
    fn effort_effect_needs_two_different_nonzero_samples() {
        // 全 0 = 看不出来（探测请求太小），不代表没生效
        assert!(!effort_visibly_differs(&[("low".into(), 0), ("max".into(), 0)]));
        // 只有一个非 0 也不够
        assert!(!effort_visibly_differs(&[("low".into(), 0), ("max".into(), 500)]));
        // 两个非 0 但一样大 → 说明档位没起作用
        assert!(!effort_visibly_differs(&[("low".into(), 500), ("max".into(), 500)]));
        // 拉开差距才算实证
        assert!(effort_visibly_differs(&[("low".into(), 1337), ("max".into(), 3616)]));
    }

    #[test]
    fn thinking_tokens_reads_the_anthropic_shape() {
        let body = serde_json::json!({
            "usage": {"output_tokens_details": {"thinking_tokens": 3616}}
        });
        assert_eq!(thinking_tokens(&body), 3616);
        assert_eq!(thinking_tokens(&serde_json::json!({"usage": {}})), 0);
    }

    #[test]
    fn all_levels_accepted_but_no_evidence_says_so_plainly() {
        // 不能因为都返回 200 就宣称「可直接用」—— 那是 over-claim
        let r = full_report();
        let line = headlines(&r).into_iter().next().unwrap();
        assert!(line.contains("未被拒"), "{line}");
        assert!(line.contains("没能确认"), "{line}");
    }

    #[test]
    fn measured_differences_upgrade_the_headline_to_confirmed() {
        let mut r = full_report();
        r.effort_thinking = vec![("low".into(), 1337), ("max".into(), 3616)];
        let line = headlines(&r).into_iter().next().unwrap();
        assert!(line.contains("已确认真实生效"), "{line}");
    }

    fn full_report() -> ProbeReport {
        ProbeReport {
            ok: true,
            validates_model_name: true,
            accepts_claude_slot: true,
            prompt_caching: true,
            accepts_1m_beta: true,
            effort_accepted: EFFORT_LEVELS.iter().map(|l| (l.to_string(), true)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_fully_capable_provider_gets_one_headline() {
        let lines = headlines(&full_report());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("五个档位"));
    }

    #[test]
    fn silent_fallback_is_called_out_first() {
        // 这是最危险的一项：用户完全不知道自己在用什么模型
        let mut r = full_report();
        r.validates_model_name = false;
        assert!(headlines(&r)[0].contains("静默回落"));
    }

    #[test]
    fn partial_effort_support_is_reported_with_the_levels() {
        let mut r = full_report();
        r.effort_accepted = vec![
            ("low".into(), true),
            ("medium".into(), false),
            ("high".into(), true),
            ("xhigh".into(), false),
            ("max".into(), false),
        ];
        let line = headlines(&r).into_iter().find(|l| l.contains("部分")).unwrap();
        assert!(line.contains("low / high"));
    }

    #[test]
    fn failed_connection_produces_no_headlines() {
        let r = ProbeReport { ok: false, message: "boom".into(), ..Default::default() };
        assert!(headlines(&r).is_empty());
    }
}

