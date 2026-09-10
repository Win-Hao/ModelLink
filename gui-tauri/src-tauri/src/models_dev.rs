//! models.dev 费率同步（§3.1 的数据来源，2026-09-10 用户拍板「完整运行时同步」）。
//!
//! 原理：models.dev 是社区维护的开源模型数据库（anomalyco/models.dev，MIT），
//! 数据以 TOML 存在仓库里、CI 编译成单个 `https://models.dev/api.json`：
//!
//! ```json
//! { "moonshotai-cn": { "name": "Moonshot AI (China)",
//!     "models": { "kimi-k2.6": { "cost": { "input": 0.95, "output": 4.0, "cache_read": 0.16 } } } } }
//! ```
//!
//! `cost` 单位就是 **USD / 百万 token** —— 与 `inferenceModelPricing` 要的单位一致，
//! 这条路不经过汇率（汇率只服务于用户按人民币手填的场景）。
//!
//! ⚠️ 这是社区数据不是厂商官方接口，条目可能滞后于真实调价，所以：
//! 用户手填的费率永远优先，同步值只填没手填过的模型。

use std::collections::HashMap;

use crate::config::{Config, ModelPricing};

pub const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";

/// 自动同步的最小间隔（6 小时）。
pub const SYNC_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// 从 models.dev 取到的单个模型信息。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModelInfo {
    /// 费率（USD/百万 token）。
    pub pricing: ModelPricing,
    /// 最大上下文（token）。None = 该条目没写。
    pub context: Option<u64>,
}

/// 拍平后的目录：models.dev 服务商 ID → 模型 ID → 模型信息。
pub type Catalog = HashMap<String, HashMap<String, ModelInfo>>;

/// 本地服务商 URL → models.dev 服务商 ID。
///
/// 内置预设逐个对上；订阅制方案（Kimi Code / 百炼 Coding Plan / 小米 Token Plan）
/// 在 models.dev 里价格是 0 —— 语义正确，订阅制本来就不按 token 计费。
pub fn provider_id_for_url(url: &str) -> Option<&'static str> {
    let u = url.to_ascii_lowercase();
    // 顺序有讲究：更具体的域名要排在通用域名前面
    let table: &[(&str, &str)] = &[
        ("api.kimi.com", "kimi-for-coding"),
        ("moonshot", "moonshotai-cn"),
        ("deepseek.com", "deepseek"),
        ("minimaxi.com", "minimax-cn"),
        ("minimax.io", "minimax"),
        ("coding.dashscope", "alibaba-coding-plan-cn"),
        ("token-plan", "alibaba-token-plan-cn"),
        ("dashscope", "alibaba-cn"),
        ("bigmodel.cn", "zhipuai"),
        ("zhipu", "zhipuai"),
        ("xiaomimimo", "xiaomi-token-plan-cn"),
    ];
    table.iter().find(|(host, _)| u.contains(host)).map(|(_, id)| *id)
}

/// 从 api.json 解析出费率表。只取 `cost`，其余字段（limit / modalities / …）一律忽略。
pub fn parse_catalog(body: &[u8]) -> Result<Catalog, String> {
    let root: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("models.dev 返回的不是合法 JSON: {e}"))?;
    let obj = root.as_object().ok_or("models.dev 返回的顶层不是对象")?;

    let mut catalog = Catalog::new();
    for (provider_id, provider) in obj {
        let Some(models) = provider.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        let mut entry = HashMap::new();
        for (model_id, model) in models {
            let Some(cost) = model.get("cost") else { continue };
            let num = |k: &str| cost.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
            let pricing = ModelPricing {
                input: num("input"),
                output: num("output"),
                cache_read: num("cache_read"),
                cache_write: num("cache_write"),
                // models.dev 一律 USD —— 标上就不会再被汇率换算一遍
                currency: "USD".to_string(),
            };
            let context = model
                .get("limit")
                .and_then(|l| l.get("context"))
                .and_then(|v| v.as_u64());
            if !pricing.is_empty() {
                entry.insert(model_id.clone(), ModelInfo { pricing, context });
            }
        }
        if !entry.is_empty() {
            catalog.insert(provider_id.clone(), entry);
        }
    }
    Ok(catalog)
}

/// 查某个服务商的某个模型的费率。
///
/// 1. 认识这个服务商 → 只在它名下查（同一个模型 ID 在不同服务商价格不同，不能串）
/// 2. 不认识（用户自定义 URL）→ 全库按模型 ID 查，**只有各家报价完全一致时才采信**，
///    有分歧就放弃 —— 猜错价格比不显示价格更糟
pub fn lookup(catalog: &Catalog, url: &str, model: &str) -> Option<ModelInfo> {
    let model = model.strip_suffix("[1m]").unwrap_or(model);
    if let Some(pid) = provider_id_for_url(url) {
        let models = catalog.get(pid)?;
        if let Some(p) = find_ci(models, model) {
            return Some(p);
        }
        // 订阅制方案：这家每个模型都是 0 —— 它按订阅收费，不按 token。
        // 这类服务商在 models.dev 里列的模型名常与用户填的对不上
        //（实测 kimi-for-coding 只列 k3 / kimi-for-coding，用户填的是 Kimi-k2.6），
        // 而查不到就意味着这个槽位退回 Anthropic 官方价 —— 假账单。
        if is_subscription_plan(models) {
            return Some(ModelInfo {
                pricing: ModelPricing {
                    input: Some(0.0),
                    output: Some(0.0),
                    cache_read: Some(0.0),
                    cache_write: Some(0.0),
                    currency: "USD".to_string(),
                },
                // 订阅制方案里列的模型名常与用户填的对不上，上下文上限无从推断
                context: None,
            });
        }
        return None;
    }
    let mut found: Option<ModelInfo> = None;
    for models in catalog.values() {
        if let Some(p) = find_ci(models, model) {
            match &found {
                Some(prev) if *prev != p => return None, // 各家报价不一致 → 不猜
                Some(_) => {}
                None => found = Some(p),
            }
        }
    }
    found
}

/// 这家服务商的每一个模型都定价为 0 → 订阅制，按 token 计费为 0。
/// 只要有一个模型是按量计价（如百炼 Coding Plan 里的 qwen3.7-max），就不适用。
fn is_subscription_plan(models: &HashMap<String, ModelInfo>) -> bool {
    !models.is_empty()
        && models.values().all(|m| {
            let p = &m.pricing;
            [p.input, p.output, p.cache_read, p.cache_write]
                .iter()
                .all(|v| v.unwrap_or(0.0) == 0.0)
        })
}

fn find_ci(models: &HashMap<String, ModelInfo>, model: &str) -> Option<ModelInfo> {
    if let Some(p) = models.get(model) {
        return Some(p.clone());
    }
    models
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(model))
        .map(|(_, v)| v.clone())
}

/// 把费率表灌进配置里所有已配置的模型，返回实际发生变化的条数。
/// 只写 `pricing_synced`；用户手填的 `pricing` 一个字节都不碰。
pub fn apply_catalog(config: &mut Config, catalog: &Catalog) -> usize {
    let mut changed = 0;
    for provider in &mut config.providers {
        let url = provider.target_url.clone();
        for m in &mut provider.models {
            if m.name.is_empty() {
                continue;
            }
            let found = lookup(catalog, &url, &m.name);
            let (pricing, context) = match found {
                Some(info) => (Some(info.pricing), info.context),
                None => (None, None),
            };
            if m.pricing_synced != pricing || m.context_limit != context {
                m.pricing_synced = pricing;
                m.context_limit = context;
                changed += 1;
            }
        }
    }
    changed
}

/// 拉取 + 解析。**不碰配置** —— 网络往返期间用户可能正在改配置，
/// 落盘一律等回到主流程、重新取写锁之后再做（见 commands::sync_pricing）。
pub async fn fetch_catalog(client: &reqwest::Client) -> Result<Catalog, String> {
    let resp = client
        .get(MODELS_DEV_API_URL)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("拉取 models.dev 失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("models.dev 返回 HTTP {}", resp.status().as_u16()));
    }
    let body = resp.bytes().await.map_err(|e| format!("读取 models.dev 响应失败: {e}"))?;
    parse_catalog(&body)
}

/// 把「后端专管」的同步费率从旧配置搬到前端回传的新配置上（按 服务商 URL + 模型名 匹配，
/// 不按下标 —— 用户可能刚拖动过顺序）。
///
/// 没有这一步就有这么个窗口：后台同步刚写完 config.json，用户手上那份草稿还是同步前的，
/// 一保存就把同步结果整个抹掉。`last_applied_*` / `port` 用的是同一套「后端专管」思路。
pub fn preserve_synced_pricing(incoming: &mut Config, current: &Config) {
    type Synced = (Option<ModelPricing>, Option<u64>);
    let mut known: HashMap<(String, String), Synced> = HashMap::new();
    for p in &current.providers {
        for m in &p.models {
            if m.pricing_synced.is_some() || m.context_limit.is_some() {
                known.insert(
                    (p.target_url.clone(), m.name.clone()),
                    (m.pricing_synced.clone(), m.context_limit),
                );
            }
        }
    }
    for p in &mut incoming.providers {
        for m in &mut p.models {
            let found = known.get(&(p.target_url.clone(), m.name.clone())).cloned();
            let (pricing, context) = found.unwrap_or((None, None));
            m.pricing_synced = pricing;
            m.context_limit = context;
        }
    }
}

/// 距上次成功同步是否已超过阈值（`synced_at` 为空 = 从没同步过）。
pub fn is_stale(synced_at: &str, now_secs: u64) -> bool {
    match synced_at.parse::<u64>() {
        Ok(t) => now_secs.saturating_sub(t) >= SYNC_INTERVAL_SECS,
        Err(_) => true,
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelEntry, Provider};

    /// api.json 的真实结构缩样（字段名与 2026-09-10 抓取的一致）。
    const SAMPLE: &[u8] = br#"{
      "moonshotai-cn": { "name": "Moonshot AI (China)", "models": {
        "kimi-k2.6": { "cost": { "input": 0.95, "output": 4.0, "cache_read": 0.16 },
                       "limit": { "context": 262144 } },
        "kimi-k3":   { "cost": { "input": 3, "output": 15, "cache_read": 0.3 },
                       "limit": { "context": 1048576 } } } },
      "kimi-for-coding": { "name": "Kimi For Coding", "models": {
        "k3": { "cost": { "input": 0, "output": 0, "cache_read": 0, "cache_write": 0 } } } },
      "deepseek": { "name": "DeepSeek", "models": {
        "deepseek-v4-pro": { "cost": { "input": 0.435, "output": 0.87 } } } },
      "no-models": { "name": "Empty" },
      "no-cost": { "name": "No cost", "models": { "x": { "limit": { "context": 1 } } } }
    }"#;

    fn catalog() -> Catalog {
        parse_catalog(SAMPLE).unwrap()
    }

    #[test]
    fn parses_cost_blocks_and_skips_entries_without_them() {
        let c = catalog();
        assert_eq!(c["deepseek"]["deepseek-v4-pro"].pricing.input, Some(0.435));
        assert_eq!(c["deepseek"]["deepseek-v4-pro"].pricing.output, Some(0.87));
        assert_eq!(c["deepseek"]["deepseek-v4-pro"].pricing.cache_read, None);
        // 没有 models / 没有 cost 的条目不进表
        assert!(!c.contains_key("no-models"));
        assert!(!c.contains_key("no-cost"));
    }

    #[test]
    fn parsed_prices_are_marked_usd_so_the_rate_never_touches_them() {
        // models.dev 一律 USD/百万 token，再除一次汇率就成了三分之一价
        let c = catalog();
        let p = &c["moonshotai-cn"]["kimi-k2.6"].pricing;
        assert_eq!(p.currency, "USD");
        assert_eq!(p.in_usd(7.2), *p);
    }

    #[test]
    fn subscription_plans_keep_their_zero_prices() {
        // 订阅制没有按 token 计费；0 是有意义的值，不能当成「没填」丢掉
        let c = catalog();
        let p = &c["kimi-for-coding"]["k3"].pricing;
        assert_eq!(p.input, Some(0.0));
        assert!(!p.is_empty());
    }

    #[test]
    fn catalog_carries_the_context_limit() {
        // 判断一个模型到底有没有 1M 上下文，靠的就是这个字段
        let c = catalog();
        assert_eq!(c["moonshotai-cn"]["kimi-k2.6"].context, Some(262_144));
        assert_eq!(c["moonshotai-cn"]["kimi-k3"].context, Some(1_048_576));
        // 没有 limit 的条目 → None（未知，不下结论）
        assert_eq!(c["deepseek"]["deepseek-v4-pro"].context, None);
    }

    #[test]
    fn apply_catalog_fills_the_context_limit_too() {
        let mut cfg = Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models: vec![ModelEntry {
                    name: "kimi-k2.6".into(),
                    to_1m: "auto".into(),
                    ..Default::default()
                }],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        apply_catalog(&mut cfg, &catalog());
        assert_eq!(cfg.providers[0].models[0].context_limit, Some(262_144));
    }

    #[test]
    fn one_million_context_is_recognised() {
        // 各家对「1M」的实际数字不一样：Kimi 是 1048576，DeepSeek / 智谱是 1000000
        assert!(!ModelEntry { context_limit: Some(262_144), ..Default::default() }.has_1m_context());
        assert!(ModelEntry { context_limit: Some(1_000_000), ..Default::default() }.has_1m_context());
        assert!(ModelEntry { context_limit: Some(1_048_576), ..Default::default() }.has_1m_context());
        // 不知道就别下结论 —— 只有「明确知道装不下」才提示用户
        assert!(ModelEntry { context_limit: None, ..Default::default() }.has_1m_context());
    }

    #[test]
    fn the_1m_switch_is_flagged_only_when_we_know_it_cannot_hold() {
        let on_small = ModelEntry {
            to_1m: "auto".into(),
            context_limit: Some(262_144),
            ..Default::default()
        };
        assert!(on_small.claims_1m_it_does_not_have());
        assert!(!ModelEntry { context_limit: Some(262_144), ..Default::default() }
            .claims_1m_it_does_not_have());
        assert!(!ModelEntry {
            to_1m: "auto".into(),
            context_limit: Some(1_048_576),
            ..Default::default()
        }
        .claims_1m_it_does_not_have());
        assert!(!ModelEntry { to_1m: "auto".into(), ..Default::default() }
            .claims_1m_it_does_not_have());
    }

    #[test]
    fn provider_urls_map_to_the_right_models_dev_ids() {
        // Kimi 订阅制与 Kimi 开放平台是两家不同的 models.dev 服务商，价格天差地别
        assert_eq!(provider_id_for_url("https://api.kimi.com/coding/"), Some("kimi-for-coding"));
        assert_eq!(provider_id_for_url("https://api.moonshot.cn/anthropic"), Some("moonshotai-cn"));
        assert_eq!(provider_id_for_url("https://api.deepseek.com/anthropic"), Some("deepseek"));
        assert_eq!(
            provider_id_for_url("https://coding.dashscope.aliyuncs.com/apps/anthropic"),
            Some("alibaba-coding-plan-cn")
        );
        assert_eq!(
            provider_id_for_url("https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic"),
            Some("alibaba-token-plan-cn")
        );
        assert_eq!(provider_id_for_url("https://open.bigmodel.cn/api/anthropic"), Some("zhipuai"));
        assert_eq!(provider_id_for_url("https://api.example.com/v1"), None);
    }

    #[test]
    fn lookup_stays_inside_the_matched_provider() {
        let c = catalog();
        // 同一个模型名在别家可能是另一个价，认识服务商时绝不跨家找
        assert_eq!(lookup(&c, "https://api.moonshot.cn/anthropic", "kimi-k2.6").unwrap().pricing.input, Some(0.95));
        // Kimi Code 是订阅制（全 0），拿到的必须是它自己的 0，不能串成 Moonshot 的 0.95
        assert_eq!(lookup(&c, "https://api.kimi.com/coding/", "kimi-k2.6").unwrap().pricing.input, Some(0.0));
        // [1m] 变体查的是裸模型名
        assert_eq!(lookup(&c, "https://api.moonshot.cn/anthropic", "kimi-k2.6[1m]").unwrap().pricing.input, Some(0.95));
        // 大小写不敏感
        assert!(lookup(&c, "https://api.moonshot.cn/anthropic", "KIMI-K2.6").is_some());
    }

    #[test]
    fn subscription_providers_price_unlisted_models_at_zero() {
        // 订阅制方案（Kimi Code / 百炼 Token Plan / 小米 Token Plan / MiniMax Token Plan）
        // 在 models.dev 里每个模型都是 0，但列的模型名未必和用户填的一致
        //（实测：kimi-for-coding 只列了 k3 / kimi-for-coding，用户填的是 Kimi-k2.6）。
        // 认出「这家所有模型都是 0」就能安全推断按 token 计费为 0 ——
        // 否则这个槽位会退回 Anthropic 官方价，也就是假账单。
        let c = catalog();
        let p = lookup(&c, "https://api.kimi.com/coding/", "Kimi-k2.6").unwrap().pricing;
        assert_eq!(p.input, Some(0.0));
        assert_eq!(p.output, Some(0.0));
        assert_eq!(p.currency, "USD");

        // 按量付费的服务商不适用：查不到就是查不到，绝不当成 0
        assert_eq!(lookup(&c, "https://api.moonshot.cn/anthropic", "查无此模型"), None);
    }

    #[test]
    fn unknown_provider_only_accepts_an_unambiguous_global_match() {
        let mut c = catalog();
        // 自定义 URL：全库唯一命中 → 采信
        assert_eq!(lookup(&c, "https://my-relay.example.com", "deepseek-v4-pro").unwrap().pricing.input, Some(0.435));
        // 两家报价不一致 → 宁可不显示，也不猜
        c.get_mut("kimi-for-coding").unwrap().insert(
            "deepseek-v4-pro".into(),
            ModelInfo {
                pricing: ModelPricing {
                    input: Some(99.0),
                    currency: "USD".into(),
                    ..Default::default()
                },
                context: None,
            },
        );
        assert_eq!(lookup(&c, "https://my-relay.example.com", "deepseek-v4-pro"), None);
        assert_eq!(lookup(&c, "https://my-relay.example.com", "查无此模型"), None);
    }

    #[test]
    fn apply_catalog_fills_synced_slot_and_reports_changes() {
        let mut cfg = Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models: vec![
                    ModelEntry { name: "kimi-k2.6".into(), ..Default::default() },
                    ModelEntry { name: "查无此模型".into(), ..Default::default() },
                    ModelEntry { name: String::new(), ..Default::default() },
                ],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let c = catalog();
        assert_eq!(apply_catalog(&mut cfg, &c), 1);
        assert_eq!(cfg.providers[0].models[0].pricing_synced.as_ref().unwrap().input, Some(0.95));
        assert_eq!(cfg.providers[0].models[1].pricing_synced, None);
        // 幂等：没变化就不算改动（避免每次同步都把用户拽进 dirty 态）
        assert_eq!(apply_catalog(&mut cfg, &c), 0);
    }

    #[test]
    fn apply_catalog_never_touches_hand_entered_pricing() {
        let manual = ModelPricing { input: Some(1.23), ..Default::default() };
        let mut cfg = Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models: vec![ModelEntry {
                    name: "kimi-k2.6".into(),
                    pricing: Some(manual.clone()),
                    ..Default::default()
                }],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        apply_catalog(&mut cfg, &catalog());
        assert_eq!(cfg.providers[0].models[0].pricing, Some(manual));
    }

    #[test]
    fn synced_pricing_survives_a_stale_draft_being_saved() {
        let synced = ModelPricing { input: Some(0.95), currency: "USD".into(), ..Default::default() };
        let mk = |sync: Option<ModelPricing>| Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models: vec![
                    ModelEntry { name: "kimi-k2.6".into(), pricing_synced: sync, ..Default::default() },
                    ModelEntry { name: "别的模型".into(), ..Default::default() },
                ],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        // 后端刚同步完；前端回传的是同步前的草稿
        let current = mk(Some(synced.clone()));
        let mut incoming = mk(None);
        preserve_synced_pricing(&mut incoming, &current);
        assert_eq!(incoming.providers[0].models[0].pricing_synced, Some(synced));
        assert_eq!(incoming.providers[0].models[1].pricing_synced, None);
    }

    #[test]
    fn preserving_matches_by_name_not_index() {
        let synced = ModelPricing { input: Some(0.95), currency: "USD".into(), ..Default::default() };
        let provider = |models: Vec<ModelEntry>| Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models,
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let current = provider(vec![
            ModelEntry { name: "a".into(), pricing_synced: Some(synced.clone()), ..Default::default() },
            ModelEntry { name: "b".into(), ..Default::default() },
        ]);
        // 用户把两行调了个个儿
        let mut incoming = provider(vec![
            ModelEntry { name: "b".into(), ..Default::default() },
            ModelEntry { name: "a".into(), ..Default::default() },
        ]);
        preserve_synced_pricing(&mut incoming, &current);
        assert_eq!(incoming.providers[0].models[0].pricing_synced, None);
        assert_eq!(incoming.providers[0].models[1].pricing_synced, Some(synced));
    }

    #[test]
    fn stale_check_drives_the_six_hour_auto_sync() {
        let now = 1_000_000u64;
        assert!(is_stale("", now), "从没同步过一定要同步");
        assert!(is_stale("坏数据", now));
        assert!(!is_stale(&(now - 60).to_string(), now));
        assert!(!is_stale(&(now - SYNC_INTERVAL_SECS + 1).to_string(), now));
        assert!(is_stale(&(now - SYNC_INTERVAL_SECS).to_string(), now));
        // 系统时钟往回跳时不应炸（saturating）
        assert!(!is_stale(&(now + 99_999).to_string(), now));
    }
}
