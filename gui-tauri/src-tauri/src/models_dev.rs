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

/// models.dev 服务商 ID → 它当前提供的模型 ID 列表（按发布日期新→旧）。
pub type ModelIndex = HashMap<String, Vec<String>>;

/// models.dev 服务商 ID → 模型 ID → 上下文上限（token）。只收写了 `limit.context` 的条目。
pub type ContextIndex = HashMap<String, HashMap<String, u64>>;

/// 从 api.json 抽出每家服务商的模型 ID 列表，供模型名输入框做补全。
///
/// 与 `parse_catalog` 分开：那个只收有 `cost` 的条目（费率表用），
/// 而补全列表要列全这家「现在提供什么」，没标价的也算。
///
/// 排序按 `release_date` 倒序 —— 用户接进来第一件事是挑当前能用的模型，
/// 最新的排最前最有用；没写日期的排最后但不丢。
pub fn parse_model_ids(body: &[u8]) -> ModelIndex {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return ModelIndex::new();
    };
    let Some(obj) = root.as_object() else {
        return ModelIndex::new();
    };
    let mut out = ModelIndex::new();
    for (provider_id, provider) in obj {
        let Some(models) = provider.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        let mut rows: Vec<(String, String)> = models
            .iter()
            .map(|(id, m)| {
                let date = m
                    .get("release_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (id.clone(), date)
            })
            .collect();
        // 日期倒序；同日期或都没日期时按 ID 升序，保证输出稳定
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        if !rows.is_empty() {
            out.insert(provider_id.clone(), rows.into_iter().map(|(id, _)| id).collect());
        }
    }
    out
}

/// 从 api.json 抽出每个模型的上下文上限，供模型选择器标注、1M 开关判断。
///
/// 费率表里也有上下文，但只收了有 `cost` 的条目；选择器列出的是这家的全部模型，
/// 没标价的也要知道它装不装得下 1M。
pub fn parse_model_contexts(body: &[u8]) -> ContextIndex {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return ContextIndex::new();
    };
    let Some(obj) = root.as_object() else {
        return ContextIndex::new();
    };
    let mut out = ContextIndex::new();
    for (provider_id, provider) in obj {
        let Some(models) = provider.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        let known: HashMap<String, u64> = models
            .iter()
            .filter_map(|(id, m)| {
                let ctx = m.get("limit")?.get("context")?.as_u64()?;
                Some((id.clone(), ctx))
            })
            .collect();
        if !known.is_empty() {
            out.insert(provider_id.clone(), known);
        }
    }
    out
}

/// 用模型清单里的上下文上限，补齐还不知道上限的模型（刚加的、改了名的、没标价的），
/// 返回补上的条数。
///
/// 只补空的，已知的不动（那是费率同步写的，两者同源）。只在这家服务商名下查；
/// 认不出的服务商（自定义 URL）不猜。
pub fn fill_known_context(config: &mut Config) -> usize {
    let contexts = &config.models_dev_context;
    let mut filled = 0;
    for p in &mut config.providers {
        let Some(known) = provider_id_for_url(&p.target_url).and_then(|pid| contexts.get(pid)) else {
            continue;
        };
        for m in &mut p.models {
            if m.context_limit.is_some() || m.name.is_empty() {
                continue;
            }
            let name = m.name.strip_suffix("[1m]").unwrap_or(&m.name);
            let found = known
                .get(name)
                .or_else(|| known.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v));
            if let Some(ctx) = found {
                m.context_limit = Some(*ctx);
                filled += 1;
            }
        }
    }
    filled
}

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
        // MiniMax 中国站在换域名：官方文档里的 API 地址已是 api.minimax.cn（api.minimaxi.com 仍可用）
        ("minimax.cn", "minimax-cn"),
        ("minimax.io", "minimax"),
        // 小米两条线地址不同（官方文档 + models.dev 的 api 字段一致）：
        // token-plan-cn.xiaomimimo.com 是 Token Plan（订阅制，全 0 价），api.xiaomimimo.com 是按量付费。
        // 2.2 之前把后者也映射到 Token Plan，按量用户的费用一律显示 0 —— 假账单。
        // 两条都必须排在通用的 "token-plan" 前面，否则小米 Token Plan 会被认成百炼。
        ("token-plan-cn.xiaomimimo", "xiaomi-token-plan-cn"),
        ("xiaomimimo", "xiaomi"),
        ("coding.dashscope", "alibaba-coding-plan-cn"),
        ("token-plan", "alibaba-token-plan-cn"),
        ("dashscope", "alibaba-cn"),
        ("bigmodel.cn", "zhipuai"),
        ("zhipu", "zhipuai"),
    ];
    table.iter().find(|(host, _)| u.contains(host)).map(|(_, id)| *id)
}

/// 我们认得的全部 models.dev 服务商 ID（`provider_id_for_url` 的值域）。
pub fn known_provider_ids() -> Vec<&'static str> {
    vec![
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
    ]
}

/// 只留我们认得的那几家 —— api.json 有 213 家，全存进 config.json 会让它从
/// 600 字节涨到 238 KB（实测），而那个文件每次编辑都要重写。
pub fn keep_known_providers<V>(index: HashMap<String, V>) -> HashMap<String, V> {
    let known = known_provider_ids();
    index.into_iter().filter(|(k, _)| known.contains(&k.as_str())).collect()
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
                },
                // 价格是服务商的属性（订阅制 = 0），上下文窗口是**模型自身**的属性 ——
                // 这家没列这个模型，去别家借它的上下文事实是成立的（借价格则不成立）。
                context: global_context(catalog, model),
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

/// 跨服务商查这个模型的上下文上限，取各家里**最宽松**的那个值。
///
/// 只对**上下文**这么做：它是模型自身的属性，同一个模型在哪家服务都是那么大；
/// 价格则相反，同一个模型在不同家可以差好几倍，绝不能这样借。
///
/// 为什么取最大值而不是要求各家完全一致：实测 26 家都列了 kimi-k2.6，绝大多数写
/// 262144，但 routing-run 写 200000、hyper 写 262000、privatemode-ai 写 256000 ——
/// 要求完全一致就永远得不出结论。而这里只需回答一个是非题「有没有 1M」，
/// 取最大值意味着**只有各家一致认为不到 1M 时才会提示用户**，宁可少提示。
pub fn global_context(catalog: &Catalog, model: &str) -> Option<u64> {
    catalog
        .values()
        .filter_map(|models| find_ci(models, model).and_then(|m| m.context))
        .max()
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
            // ⚠️ 查不到就**保持原值**，绝不清空。
            // models.dev 某次返回空表 / 改了结构 / 下掉一个模型条目，都会让这里查不到；
            // 一旦清空，费率行随之消失，引擎立刻退回按 Anthropic 官方价估算 —— 假账单。
            // 价格略旧远好过假账单。（改了模型名的情形由 preserve_synced_pricing 负责丢弃。）
            let Some(info) = lookup(catalog, &url, &m.name) else {
                continue;
            };
            if m.pricing_synced.as_ref() != Some(&info.pricing) || m.context_limit != info.context {
                m.pricing_synced = Some(info.pricing);
                m.context_limit = info.context;
                changed += 1;
            }
        }
    }
    changed
}

/// 拉取 + 解析。**不碰配置** —— 网络往返期间用户可能正在改配置，
/// 落盘一律等回到主流程、重新取写锁之后再做（见 commands::sync_pricing）。
pub async fn fetch_catalog(
    client: &reqwest::Client,
) -> Result<(Catalog, ModelIndex, ContextIndex), String> {
    fetch_catalog_from(client, MODELS_DEV_API_URL).await
}

/// 同上，地址可注入（单测用）。
pub async fn fetch_catalog_from(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Catalog, ModelIndex, ContextIndex), String> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("拉取 models.dev 失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("models.dev 返回 HTTP {}", resp.status().as_u16()));
    }
    let body = resp.bytes().await.map_err(|e| format!("读取 models.dev 响应失败: {e}"))?;
    Ok((
        parse_catalog(&body)?,
        keep_known_providers(parse_model_ids(&body)),
        keep_known_providers(parse_model_contexts(&body)),
    ))
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
                       "release_date": "2026-04-21",
                       "limit": { "context": 262144 } },
        "kimi-k3":   { "cost": { "input": 3, "output": 15, "cache_read": 0.3 },
                       "release_date": "2026-07-16",
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
        assert_eq!(c["moonshotai-cn"]["kimi-k2.6"].pricing.input, Some(0.95));
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
    fn model_ids_are_listed_newest_first() {
        // 选择器里最有用的排序是「最新的排最前」—— 用户接进来第一件事是挑当前能用的模型
        let idx = parse_model_ids(SAMPLE);
        assert_eq!(idx["moonshotai-cn"], vec!["kimi-k3", "kimi-k2.6"]);
        assert_eq!(idx["kimi-for-coding"], vec!["k3"]);
    }

    #[test]
    fn only_providers_we_can_map_are_kept() {
        // api.json 有 213 家；全存进 config.json 会让它从 600 字节涨到 238 KB
        let idx = keep_known_providers(parse_model_ids(SAMPLE));
        assert!(idx.contains_key("moonshotai-cn"));
        assert!(idx.contains_key("kimi-for-coding"));
        assert!(!idx.contains_key("no-cost"), "认不出的服务商不该留");
    }

    #[test]
    fn every_mapped_provider_id_is_in_the_known_list() {
        // 两处一旦漂移，某家的补全列表就会静默消失
        for url in [
            "https://api.kimi.com/coding/",
            "https://api.moonshot.cn/anthropic",
            "https://api.deepseek.com/anthropic",
            "https://api.minimaxi.com/anthropic",
            "https://coding.dashscope.aliyuncs.com/apps/anthropic",
            "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
            "https://open.bigmodel.cn/api/anthropic",
            "https://api.minimax.cn/anthropic",
            "https://api.xiaomimimo.com/anthropic",
            "https://token-plan-cn.xiaomimimo.com/anthropic",
        ] {
            let pid = provider_id_for_url(url).unwrap_or_else(|| panic!("{url} 没映射"));
            assert!(known_provider_ids().contains(&pid), "{pid} 不在 known 列表里");
        }
    }

    #[test]
    fn model_ids_include_entries_without_a_cost_block() {
        // 补全列表要列全这家「现在提供什么」，没标价的条目也算
        let body = br#"{"x": {"models": {
            "with-cost": {"cost": {"input": 1}, "release_date": "2026-01-01"},
            "no-cost":   {"release_date": "2026-02-01"}
        }}}"#;
        assert_eq!(parse_model_ids(body)["x"], vec!["no-cost", "with-cost"]);
    }

    #[test]
    fn model_ids_without_release_date_sort_last_but_are_kept() {
        let body = br#"{"x": {"models": {
            "dated": {"release_date": "2026-01-01"},
            "undated": {}
        }}}"#;
        assert_eq!(parse_model_ids(body)["x"], vec!["dated", "undated"]);
    }

    #[test]
    fn model_contexts_cover_entries_without_a_cost_block() {
        // 选择器要标出每个模型的上下文，没标价的也算；没写 limit 的就是不知道
        let ctx = parse_model_contexts(SAMPLE);
        assert_eq!(ctx["moonshotai-cn"]["kimi-k3"], 1_048_576);
        assert_eq!(ctx["no-cost"]["x"], 1);
        assert!(!ctx.contains_key("deepseek"), "没写 limit 的服务商不该出现");
        assert!(parse_model_contexts(b"not json").is_empty());
    }

    fn cfg_for(url: &str, models: Vec<ModelEntry>) -> Config {
        Config {
            providers: vec![Provider {
                target_url: url.into(),
                api_key: "k".into(),
                models,
                thinking_effort: String::new(),
            }],
            models_dev_context: keep_known_providers(parse_model_contexts(SAMPLE)),
            ..Default::default()
        }
    }

    #[test]
    fn fill_known_context_only_fills_the_blanks() {
        // 刚从选择器里挑的模型：还没同步过，1M 开关却要马上知道它装不装得下
        let mut cfg = cfg_for(
            "https://api.moonshot.cn/anthropic",
            vec![
                ModelEntry { name: "KIMI-K3".into(), ..Default::default() },
                ModelEntry { name: "kimi-k2.6[1m]".into(), ..Default::default() },
                // 已知的不动
                ModelEntry { name: "kimi-k3".into(), context_limit: Some(7), ..Default::default() },
                ModelEntry { name: "查无此模型".into(), ..Default::default() },
            ],
        );
        assert_eq!(fill_known_context(&mut cfg), 2);
        let got: Vec<_> = cfg.providers[0].models.iter().map(|m| m.context_limit).collect();
        assert_eq!(got, vec![Some(1_048_576), Some(262_144), Some(7), None]);
    }

    #[test]
    fn fill_known_context_does_not_guess_for_custom_urls() {
        let mut cfg = cfg_for(
            "https://my-relay.example.com",
            vec![ModelEntry { name: "kimi-k3".into(), ..Default::default() }],
        );
        assert_eq!(fill_known_context(&mut cfg), 0);
        assert_eq!(cfg.providers[0].models[0].context_limit, None);
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
        use crate::config::context_holds_1m;
        // 各家对「1M」的实际数字不一样：Kimi 是 1048576，DeepSeek / 智谱是 1000000
        assert!(!context_holds_1m(Some(262_144)));
        assert!(context_holds_1m(Some(1_000_000)));
        assert!(context_holds_1m(Some(1_048_576)));
        // 不知道就别下结论 —— 只有「明确知道装不下」才提示用户
        assert!(context_holds_1m(None));
    }

    #[test]
    fn the_1m_switch_is_flagged_only_when_we_know_it_cannot_hold() {
        use crate::config::claims_1m_without_it;
        assert!(claims_1m_without_it("auto", Some(262_144)));
        // 关着的、够大的、未知的都不该报
        assert!(!claims_1m_without_it("", Some(262_144)));
        assert!(!claims_1m_without_it("auto", Some(1_048_576)));
        assert!(!claims_1m_without_it("auto", None));
    }

    #[test]
    fn context_limit_falls_back_across_providers_but_price_does_not() {
        // 上下文窗口是**模型自身**的属性，价格是**服务商**的属性 —— 两者的回退规则不同。
        // 实例：Kimi Code（订阅制）下没列 Kimi-k2.6，但 moonshotai-cn 下写着 262144。
        let c = catalog();
        let got = lookup(&c, "https://api.kimi.com/coding/", "kimi-k2.6").unwrap();
        // 价格用的是订阅制那家自己的 0，绝不串成 Moonshot 的 0.95
        assert_eq!(got.pricing.input, Some(0.0));
        // 上下文借用模型级事实
        assert_eq!(got.context, Some(262_144));
    }

    #[test]
    fn cross_provider_context_takes_the_most_generous_claim() {
        // 实测各家对同一模型的上下文写法有出入（262144 / 262000 / 256000 / 200000）。
        // 取最大值 → 只有各家一致认为不到 1M 时才会提示用户，宁可少提示。
        let mut c = catalog();
        c.get_mut("deepseek").unwrap().get_mut("deepseek-v4-pro").unwrap().context = Some(200_000);
        c.get_mut("kimi-for-coding").unwrap().insert(
            "deepseek-v4-pro".into(),
            ModelInfo { pricing: ModelPricing::default(), context: Some(1_000_000) },
        );
        assert_eq!(global_context(&c, "deepseek-v4-pro"), Some(1_000_000));
        // 谁都没写就是不知道
        assert_eq!(global_context(&c, "查无此模型"), None);
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

    /// 小米按量付费（api.xiaomimimo.com）和 Token Plan（token-plan-cn.xiaomimimo.com）是两条线。
    /// 2.2 之前前者被当成 Token Plan，费用全显示 0；Token Plan 地址又会被通用的 "token-plan" 认成百炼。
    #[test]
    fn xiaomi_pay_as_you_go_and_token_plan_are_different_providers() {
        assert_eq!(provider_id_for_url("https://api.xiaomimimo.com/anthropic"), Some("xiaomi"));
        assert_eq!(
            provider_id_for_url("https://token-plan-cn.xiaomimimo.com/anthropic"),
            Some("xiaomi-token-plan-cn")
        );
        let c = parse_catalog(
            br#"{
              "xiaomi": {"models": {"mimo-v2.5-pro": {"cost": {"input": 0.435, "output": 0.87}}}},
              "xiaomi-token-plan-cn": {"models": {"mimo-v2.5-pro": {"cost": {"input": 0, "output": 0}}}}
            }"#,
        )
        .unwrap();
        let payg = lookup(&c, "https://api.xiaomimimo.com/anthropic", "mimo-v2.5-pro").unwrap();
        assert_eq!(payg.pricing.input, Some(0.435), "按量付费不能拿到订阅制的 0 价");
        let plan = lookup(&c, "https://token-plan-cn.xiaomimimo.com/anthropic", "mimo-v2.5-pro").unwrap();
        assert_eq!(plan.pricing.input, Some(0.0));
    }

    /// 照 MiniMax 新文档填 api.minimax.cn 的用户，要和老地址拿到同一家的费率与模型清单。
    #[test]
    fn minimax_new_china_domain_maps_like_the_old_one() {
        assert_eq!(provider_id_for_url("https://api.minimax.cn/anthropic"), Some("minimax-cn"));
        assert_eq!(provider_id_for_url("https://api.minimaxi.com/anthropic"), Some("minimax-cn"));
        assert_eq!(provider_id_for_url("https://api.minimax.io/anthropic"), Some("minimax"));
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
    fn synced_pricing_survives_a_stale_draft_being_saved() {
        let synced = ModelPricing { input: Some(0.95), ..Default::default() };
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
        let synced = ModelPricing { input: Some(0.95), ..Default::default() };
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

    #[tokio::test]
    async fn unreachable_models_dev_degrades_instead_of_breaking() {
        // 拉不到就保持上次同步的值（降级到「价格略旧」，而不是「没有价格」）。
        // 用一个必定连不上的地址模拟断网 / 被墙。
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(300))
            .build()
            .unwrap();
        // 本机可能挂着 HTTP 代理，拿到的是 502 而不是连接被拒 —— 两种都算失败，
        // 关键是**返回 Err 而不是 panic / 半个表**，调用方据此保持旧费率。
        let err = fetch_catalog_from(&client, "http://127.0.0.1:1/api.json").await.unwrap_err();
        assert!(!err.is_empty(), "失败必须带上原因，日志里要看得懂");
    }

    #[test]
    fn garbage_payload_is_rejected_rather_than_half_parsed() {
        assert!(parse_catalog(b"not json").is_err());
        assert!(parse_catalog(b"[1,2,3]").is_err(), "顶层不是对象也该报错");
        // 合法但空 → 空表，不是错误
        assert!(parse_catalog(b"{}").unwrap().is_empty());
    }

    #[test]
    fn an_empty_catalog_never_wipes_existing_prices() {
        // 万一 models.dev 某次返回了个空表，不能把用户已有的费率清掉
        let synced = ModelPricing { input: Some(0.95), ..Default::default() };
        let mut cfg = Config {
            providers: vec![Provider {
                target_url: "https://api.moonshot.cn/anthropic".into(),
                api_key: "k".into(),
                models: vec![ModelEntry {
                    name: "kimi-k2.6".into(),
                    pricing_synced: Some(synced.clone()),
                    context_limit: Some(262_144),
                    ..Default::default()
                }],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        assert_eq!(apply_catalog(&mut cfg, &Catalog::new()), 0, "空表不该算作变更");
        assert_eq!(cfg.providers[0].models[0].pricing_synced, Some(synced));
        assert_eq!(cfg.providers[0].models[0].context_limit, Some(262_144));
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
