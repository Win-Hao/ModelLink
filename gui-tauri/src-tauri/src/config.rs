//! 配置数据模型、读写与槽位解析（平移 v1 claude-model-proxy main.rs:19-228）。
//!
//! ⚠️ 兼容红线（docs/gui-rebuild-tauri.md §3）：
//! - `~/.claude-model-proxy/config.json` 路径与 serde 格式不变
//! - 原子写入（tmp+rename）、unix 0600
//! - 8 槽位映射、`[1m]` 变体
//!
//! 平移不重写：除模块化拆分与可测试性抽取外，禁止任何行为改动。
//!
//! 2.1-A 有意偏离 v1 的一处（design-2.1-follow-desktop.md §3.3）：
//! 未匹配的槽位不再静默回落到第一个模型，改为返回 `ResolveError::UnmappedSlot`
//! （旧行为保留在 `compat_fallback` 开关后，默认关）。
// v1 原样平移的代码保持逐字节一致，不做 clippy 风格改写（红线 #4）
#![allow(clippy::ptr_arg, clippy::manual_strip)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 2.1-B §3.6：8 → 20。官方 `inferenceModels` 上限是 200 条，20 留足余量。
pub const MAX_MODELS: usize = 20;

/// 槽位池（§2.1）。顺序即分配优先级 —— Claude Desktop 内有一张按**模型 ID 精确匹配**
/// 的硬编码表（`Vwt`）决定模型选择器里出不出现推理强度 / 思考模式选项，排在前面的
/// 槽位档位更全：
///
/// | 槽位 | 桌面端 effort 档位 | 思考模式 |
/// |---|---|---|
/// | opus-5 / sonnet-5 / opus-4-8 / opus-4-7 | low/medium/high/xhigh/max | auto |
/// | opus-4-6 | low/medium/high/max | extended |
/// | sonnet-4-6 | low/medium/high/max | auto |
/// | sonnet-4-5 / haiku-4-5 | 无 | extended |
///
/// 换掉 2.0 的 8 个 legacy 槽位（`claude-3-opus-latest` 等）—— 那些一个都不在 `Vwt` 表里，
/// 桌面端根本不渲染推理强度选择器，这正是 v1 只能在代理里硬注入 effort 的原因。
///
/// ⚠️ 这里只存槽位 ID。档位表是**桌面端 UI 的事**，ModelLink 不留副本 ——
/// §3.11.5：槽位名是借用来的 Claude 型号名，与背后真实上游模型毫无关系，
/// 发给上游的任何思考参数都必须查能力表，绝不能从槽位名推断。
pub const SLOT_POOL: &[&str] = &[
    // 一线：5 档 effort + auto 模式
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    // 二线：4 档
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    // 三线：仅 extended 模式，无 effort
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
];

/// 槽位池代号 —— **换槽位池时必须一并改这个值**。
///
/// config.json 里不存 slot（`flatten_config` 时按顺序现算），所以换池子不需要迁移
/// 配置文件；但用户 Claude Desktop 里写着的还是旧槽位，不重新应用就用不上新选择器。
/// 池代号进 `canonical_hash` 保证升级后状态机变 dirty，用户会被提示去点「应用」。
pub const SLOT_POOL_VERSION: &str = "2.1";

/// 第 `index` 个模型（0-based）占的槽位。池子用完后走 `claude-ml-{n}` 溢出层：
/// 无 effort 选择器的纯占位名，可无限扩，且含 "claude" 能过桌面端的名字过滤器（§1.3）。
pub fn slot_id(index: usize) -> String {
    match SLOT_POOL.get(index) {
        Some(id) => (*id).to_string(),
        None => format!("claude-ml-{}", index - SLOT_POOL.len() + 1),
    }
}

pub const DEFAULT_PORT: u16 = 5678;

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn is_default_port(p: &u16) -> bool {
    *p == DEFAULT_PORT
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// SSE 心跳间隔（§3.2）。取 15s：远小于引擎约 5 分钟的「无声连接」判死线，
/// 又不至于把日志刷满。
///
/// 不做成设置项 —— 用户没有任何依据判断该填几秒，而这个值也没有副作用
/// （只在上游沉默时往下游写一行 SSE 注释）。
pub const HEARTBEAT_SECS: u64 = 15;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// 2.0 新增（design.md §8 应用状态机）：上次「应用到 Claude Desktop」的配置摘要。
    /// 空值不序列化 —— 未用过应用功能时文件输出与 v1 格式完全一致。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_applied_hash: String,
    /// 2.0 新增：上次应用的时间（Unix 秒，字符串）。空值不序列化，同上。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_applied_at: String,
    /// 2.0 新增（2026-07-14 用户拍板）：代理监听端口，后端专管（set_port 命令热切换）。
    /// 默认 5678 时不序列化 —— 红线 #1 对默认值成立，老用户文件格式不变。
    #[serde(default = "default_port", skip_serializing_if = "is_default_port")]
    pub port: u16,
    /// 2.1-B 新增（§2.2）：上次「应用」时用的槽位池代号。空 = 2.1 之前应用过（或从没应用过），
    /// 前端据此把「升级导致的 dirty」和「用户改了配置」区分开，给出对应的提示语。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_applied_pool: String,
    /// 2.1-B 新增：启动时自动从 models.dev 同步费率（6 小时阈值）。默认开。
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub pricing_auto_sync: bool,
    /// 上次成功同步的时间（Unix 秒，字符串）。空 = 从没同步过。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pricing_synced_at: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            last_applied_hash: String::new(),
            last_applied_at: String::new(),
            port: DEFAULT_PORT,
            last_applied_pool: String::new(),
            pricing_auto_sync: true,
            pricing_synced_at: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Provider {
    #[serde(default)]
    pub target_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    #[serde(default)]
    pub thinking_effort: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ModelEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub to_1m: String,
    /// 从 models.dev 同步来的费率（USD/百万 token），由 `models_dev::apply_catalog` 维护，
    /// 用户不直接编辑。手填的 `pricing` 一旦有内容就完全接管，不与它逐字段合并 ——
    /// 同步值是美元、手填可能是人民币，混在一行里就是两种币种相加。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_synced: Option<ModelPricing>,
    /// 2.1 新增：上游该模型的最大上下文（token），由 models.dev 同步填，用户不编辑。
    /// 用来判断「开着 1M 但这个模型根本装不下」。None = 未知，不下结论。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
}

/// 「1M 上下文」的判定门槛。各家给的数字不一样：Kimi 是 1048576，
/// DeepSeek / 智谱 / 百炼是 1000000 —— 取 1000000 作为下限。
pub const ONE_M_CONTEXT: u64 = 1_000_000;


/// 这个上下文上限装不装得下 1M。**未知（None）时返回 true** ——
/// 只有明确知道装不下才提示用户，绝不因为 models.dev 缺一条数据就去质疑用户的设置。
pub fn context_holds_1m(limit: Option<u64>) -> bool {
    limit.map(|c| c >= ONE_M_CONTEXT).unwrap_or(true)
}

/// 开着 1M 开关，但已知这个模型装不下。
///
/// 后果是静默的：引擎会剥掉 `[1m]` 后缀改发 `anthropic-beta: context-1m-2025-08-07`
/// 头（实测代理从没收到过带 `[1m]` 的请求），多数上游照收不误但仍按自己的上限截断 ——
/// 用户以为有 1M，实际没有。
pub fn claims_1m_without_it(to_1m: &str, limit: Option<u64>) -> bool {
    !to_1m.is_empty() && !context_holds_1m(limit)
}

impl ModelEntry {
    /// 实际写进网关的费率（全部来自 models.dev 同步 —— 没有手填入口了）。
    pub fn effective_pricing(&self) -> Option<&ModelPricing> {
        self.pricing_synced.as_ref().filter(|p| !p.is_empty())
    }
}

/// 单个模型的费率（§3.1）。单位固定 **USD / 百万 token** —— 与
/// `inferenceModelPricing` 要求的单位一致，也与 models.dev 的 `cost` 一致，
/// 全程不经过任何汇率换算。
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

impl ModelPricing {
    /// 一个字段都没填 = 等于没配（不写覆盖行）。
    pub fn is_empty(&self) -> bool {
        self.input.is_none()
            && self.output.is_none()
            && self.cache_read.is_none()
            && self.cache_write.is_none()
    }
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude-model-proxy")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> Config {
    let path = config_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Config::default()
    }
}

pub fn friendly_write_error(e: &std::io::Error, path: &PathBuf) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied =>
            format!("Permission denied: {}. Please check folder permissions or try running as administrator.", path.display()),
        std::io::ErrorKind::NotFound =>
            format!("Path not found: {}. Please ensure the parent directory exists.", path.display()),
        _ if e.raw_os_error() == Some(32) || e.raw_os_error() == Some(33) =>
            format!("File is locked: {}. Please close Claude Desktop first and try again.", path.display()),
        _ => format!("Write failed ({}): {}", path.display(), e),
    }
}

pub fn write_with_retry(path: &PathBuf, data: &str) -> Result<(), String> {
    for attempt in 0..3 {
        match std::fs::write(path, data) {
            Ok(()) => return Ok(()),
            Err(e) if (e.raw_os_error() == Some(32) || e.raw_os_error() == Some(33)) && attempt < 2 => {
                eprintln!("[write] file locked, retrying in 1s...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => return Err(friendly_write_error(&e, path)),
        }
    }
    Err(format!("Failed after retries: {}", path.display()))
}

pub fn save_config_file(config: &Config) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!("Cannot create config directory: {}. Permission denied.", dir.display())
        } else {
            format!("Cannot create config directory: {}", e)
        }
    })?;
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let target = config_path();
    let tmp = target.with_extension("json.tmp");
    write_with_retry(&tmp, &data)?;
    std::fs::rename(&tmp, &target).map_err(|e| friendly_write_error(&e, &target))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 规范化配置摘要（design.md §8）：对 providers + port 的规范化 JSON（稳定键序）
/// 取 FNV-1a 64。不依赖第三方 crate，跨版本稳定 —— 该值持久化在 config.json 里。
/// port 参与哈希（改端口须重新应用）。槽位池代号也参与（§2.2：换池子后必须重新应用）。
/// compat_fallback 只影响代理自身的路由行为、不改写 Claude Desktop 的任何键，
/// 故**不**参与哈希（切它不该提示「需重新应用」）。
pub fn canonical_hash(config: &Config) -> String {
    canonical_hash_with_pool(config, SLOT_POOL_VERSION)
}

/// 同上，槽位池代号可注入 —— 仅为单测能验证「换池子会让老 hash 失效」。
pub fn canonical_hash_with_pool(config: &Config, pool_version: &str) -> String {
    let canon = Config {
        providers: config.providers.clone(),
        port: config.port,
        ..Default::default()
    };
    let json = serde_json::to_string(&canon).unwrap_or_default();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in json.as_bytes().iter().chain(pool_version.as_bytes()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    pub model: String,
    pub target_url: String,
    pub api_key: String,
    pub thinking_effort: String,
}

/// 槽位解析失败（§3.3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// 请求的槽位没有映射到任何服务商模型 —— 代理返回 400，不再静默回落。
    /// 携带的是去掉 `[1m]` 后缀的裸槽位名（那才是需要用户去配的东西）。
    UnmappedSlot(String),
}

impl FlatEntry {
    /// 见 `claims_1m_without_it`；这里只针对**真正写进 Claude 的**槽位。
    pub fn claims_1m_it_does_not_have(&self) -> bool {
        claims_1m_without_it(&self.to_1m, self.context_limit)
    }
}

pub struct FlatEntry {
    pub slot: String,
    pub name: String,
    pub to_1m: String,
    pub url: String,
    pub key: String,
    pub thinking_effort: String,
    pub pricing: Option<ModelPricing>,
    pub context_limit: Option<u64>,
}

pub fn flatten_config(config: &Config) -> Vec<FlatEntry> {
    let mut result = Vec::new();
    let mut count = 0;
    for provider in &config.providers {
        for m in &provider.models {
            if count < MAX_MODELS && !m.name.is_empty() {
                result.push(FlatEntry {
                    slot: slot_id(count),
                    name: m.name.clone(),
                    to_1m: m.to_1m.clone(),
                    url: provider.target_url.clone(),
                    key: provider.api_key.clone(),
                    thinking_effort: provider.thinking_effort.clone(),
                    pricing: m.effective_pricing().cloned(),
                    context_limit: m.context_limit,
                });
                count += 1;
            }
        }
    }
    result
}

/// 槽位 → 上游模型。未匹配时返回 `Err(UnmappedSlot)`（§3.3）：
///
/// v1 在这里回落到 flat 里的第一个模型，只打一行 eprintln 就照发不误。叠加上游的
/// 同类行为更糟 —— 实测 Kimi `/coding/` 端点对 `banana`、空字符串、`claude-opus-5`
/// 一律 200 直接给默认模型。两层静默回落之下，用户完全不知道自己在用什么模型。
///
pub fn resolve_model(model: &str, config: &Config) -> Result<ResolvedModel, ResolveError> {
    let (base, is_1m) = if model.ends_with("[1m]") {
        (&model[..model.len() - 4], true)
    } else {
        (model, false)
    };

    let flat = flatten_config(config);
    for e in &flat {
        if base == e.slot {
            let resolved = if is_1m && !e.to_1m.is_empty() {
                format!("{}[1m]", e.name)
            } else {
                e.name.clone()
            };
            return Ok(ResolvedModel {
                model: resolved,
                target_url: e.url.clone(),
                api_key: e.key.clone(),
                thinking_effort: e.thinking_effort.clone(),
            });
        }
    }

    Err(ResolveError::UnmappedSlot(base.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, to_1m: &str) -> ModelEntry {
        ModelEntry { name: name.into(), to_1m: to_1m.into(), ..Default::default() }
    }

    fn provider(url: &str, key: &str, models: Vec<ModelEntry>, te: &str) -> Provider {
        Provider {
            target_url: url.into(),
            api_key: key.into(),
            models,
            thinking_effort: te.into(),
        }
    }

    fn sample_config() -> Config {
        Config {
            providers: vec![
                provider(
                    "https://a.example.com",
                    "key-a",
                    vec![model("model-a1", "auto"), model("model-a2", "")],
                    "max",
                ),
                provider("https://b.example.com", "key-b", vec![model("model-b1", "")], "off"),
            ],
            ..Default::default()
        }
    }

    // ---- §2.1 SLOT_POOL / §3.6 放开上限 ----

    #[test]
    fn slot_pool_matches_claude_desktop_effort_table() {
        // 顺序即分配优先级：一线（5 档）→ 二线（4 档）→ 三线（无 effort）
        assert_eq!(
            SLOT_POOL,
            [
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-opus-4-8",
                "claude-opus-4-7",
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                "claude-sonnet-4-5",
                "claude-haiku-4-5",
            ]
        );
    }

    #[test]
    fn slot_id_falls_through_to_overflow_layer() {
        assert_eq!(slot_id(0), "claude-opus-5");
        assert_eq!(slot_id(7), "claude-haiku-4-5");
        // 第 9 个模型起走溢出层，从 1 开始编号
        assert_eq!(slot_id(8), "claude-ml-1");
        assert_eq!(slot_id(19), "claude-ml-12");
    }

    #[test]
    fn overflow_slots_survive_the_name_filter() {
        // §1.3：模型名必须含 claude|opus|sonnet|haiku|fable|mythos|anthropic
        // 之一，否则 Claude Desktop 的名字过滤器会把整条删掉
        for i in 0..MAX_MODELS {
            let id = slot_id(i);
            assert!(
                ["claude", "opus", "sonnet", "haiku", "fable", "mythos", "anthropic"]
                    .iter()
                    .any(|k| id.contains(k)),
                "槽位 {} 会被名字过滤器删掉",
                id
            );
        }
    }

    #[test]
    fn flatten_caps_at_twenty_and_uses_overflow_slots() {
        let models: Vec<ModelEntry> = (0..25).map(|i| model(&format!("m{i}"), "")).collect();
        let cfg = Config {
            providers: vec![provider("https://a.example.com", "k", models, "")],
            ..Default::default()
        };
        let flat = flatten_config(&cfg);
        assert_eq!(flat.len(), 20);
        assert_eq!(flat[0].slot, "claude-opus-5");
        assert_eq!(flat[7].slot, "claude-haiku-4-5");
        assert_eq!(flat[8].slot, "claude-ml-1");
        assert_eq!(flat[19].slot, "claude-ml-12");
    }

    // ---- flatten_config ----

    #[test]
    fn flatten_assigns_slots_in_order_across_providers() {
        let flat = flatten_config(&sample_config());
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].slot, "claude-opus-5");
        assert_eq!(flat[0].name, "model-a1");
        assert_eq!(flat[0].url, "https://a.example.com");
        assert_eq!(flat[0].thinking_effort, "max");
        assert_eq!(flat[1].slot, "claude-sonnet-5");
        assert_eq!(flat[1].name, "model-a2");
        assert_eq!(flat[2].slot, "claude-opus-4-8");
        assert_eq!(flat[2].name, "model-b1");
        assert_eq!(flat[2].key, "key-b");
        assert_eq!(flat[2].thinking_effort, "off");
    }

    #[test]
    fn flatten_skips_empty_names_without_consuming_slots() {
        let cfg = Config {
            providers: vec![provider(
                "https://a.example.com",
                "k",
                vec![model("", "auto"), model("real", "")],
                "",
            )],
            ..Default::default()
        };
        let flat = flatten_config(&cfg);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].slot, "claude-opus-5");
        assert_eq!(flat[0].name, "real");
    }

    // ---- resolve_model ----

    #[test]
    fn resolve_matches_slot_to_provider_model() {
        let cfg = sample_config();
        let r = resolve_model("claude-opus-4-8", &cfg).unwrap();
        assert_eq!(r.model, "model-b1");
        assert_eq!(r.target_url, "https://b.example.com");
        assert_eq!(r.api_key, "key-b");
        assert_eq!(r.thinking_effort, "off");
    }

    #[test]
    fn resolve_1m_suffix_maps_when_to_1m_set() {
        let cfg = sample_config();
        let r = resolve_model("claude-opus-5[1m]", &cfg).unwrap();
        assert_eq!(r.model, "model-a1[1m]");
    }

    #[test]
    fn resolve_1m_suffix_dropped_when_to_1m_empty() {
        let cfg = sample_config();
        let r = resolve_model("claude-sonnet-5[1m]", &cfg).unwrap();
        assert_eq!(r.model, "model-a2");
    }

    // ---- §3.3 未映射槽位：不再静默回落 ----

    #[test]
    fn resolve_unknown_model_reports_unmapped_slot() {
        let cfg = sample_config();
        assert_eq!(
            resolve_model("claude-9-nonexistent", &cfg),
            Err(ResolveError::UnmappedSlot("claude-9-nonexistent".into()))
        );
    }

    #[test]
    fn resolve_unmapped_slot_strips_1m_suffix_in_error() {
        let cfg = sample_config();
        // 报出去的是裸槽位名 —— 那才是用户要在 ModelLink 里配的东西
        assert_eq!(
            resolve_model("claude-9-nonexistent[1m]", &cfg),
            Err(ResolveError::UnmappedSlot("claude-9-nonexistent".into()))
        );
    }

    #[test]
    fn resolve_slot_beyond_configured_models_is_unmapped() {
        // 配置里只有 3 个模型，第 4 个槽位没人认领
        let cfg = sample_config();
        assert_eq!(
            resolve_model("claude-opus-4-7", &cfg),
            Err(ResolveError::UnmappedSlot("claude-opus-4-7".into()))
        );
    }

    #[test]
    fn resolve_empty_config_reports_unmapped_slot() {
        let cfg = Config::default();
        assert_eq!(
            resolve_model("claude-opus-5", &cfg),
            Err(ResolveError::UnmappedSlot("claude-opus-5".into()))
        );
    }

    // ---- serde 格式兼容（红线 #2） ----

    #[test]
    fn serialize_without_hash_matches_v1_format_exactly() {
        let cfg = Config {
            providers: vec![provider(
                "https://api.example.com",
                "sk-test",
                vec![model("m1", "auto")],
                "",
            )],
            ..Default::default()
        };
        let out = serde_json::to_string_pretty(&cfg).unwrap();
        // v1 输出形状：仅 providers 一个顶层键；模型条目为 { name, to_1m }
        let expected = r#"{
  "providers": [
    {
      "target_url": "https://api.example.com",
      "api_key": "sk-test",
      "models": [
        {
          "name": "m1",
          "to_1m": "auto"
        }
      ],
      "thinking_effort": ""
    }
  ]
}"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn deserialize_v1_file_and_tolerates_missing_fields() {
        let v1 = r#"{"providers":[{"target_url":"https://x.example.com","api_key":"k","models":[{"name":"m"}]}]}"#;
        let cfg: Config = serde_json::from_str(v1).unwrap();
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].models[0].name, "m");
        assert_eq!(cfg.providers[0].models[0].to_1m, "");
        assert_eq!(cfg.providers[0].thinking_effort, "");
        assert_eq!(cfg.last_applied_hash, "");
    }

    // ---- canonical_hash（应用状态机 dirty 判定） ----

    #[test]
    fn canonical_hash_ignores_last_applied_hash_field() {
        let mut a = sample_config();
        let mut b = sample_config();
        a.last_applied_hash = "".into();
        b.last_applied_hash = "something-else".into();
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn canonical_hash_changes_on_any_provider_edit() {
        let base = sample_config();
        let h0 = canonical_hash(&base);

        let mut c1 = sample_config();
        c1.providers[0].api_key = "key-a2".into();
        let mut c2 = sample_config();
        c2.providers[0].models[0].to_1m = "".into();
        let mut c3 = sample_config();
        c3.providers[1].thinking_effort = "high".into();

        assert_ne!(h0, canonical_hash(&c1));
        assert_ne!(h0, canonical_hash(&c2));
        assert_ne!(h0, canonical_hash(&c3));
        // 同内容必同值（稳定性）
        assert_eq!(h0, canonical_hash(&sample_config()));
    }

    // ---- §3.1 / §五① 费率与汇率换算 ----

    #[test]
    fn pricing_is_absent_by_default_and_keeps_v1_file_shape() {
        let cfg: Config = serde_json::from_str(
            r#"{"providers":[{"target_url":"u","api_key":"k","models":[{"name":"m"}]}]}"#,
        )
        .unwrap();
        assert_eq!(cfg.providers[0].models[0].pricing_synced, None);
        // 没填费率 / 用默认汇率时，写出去的文件与 2.0 一模一样
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(!out.contains("pricing"), "{out}");
    }

    #[test]
    fn pricing_round_trips_with_only_the_fields_that_were_filled() {
        let json = r#"{"input":4.0,"output":16.0}"#;
        let pr: ModelPricing = serde_json::from_str(json).unwrap();
        assert_eq!(pr.input, Some(4.0));
        assert_eq!(pr.output, Some(16.0));
        assert_eq!(pr.cache_read, None);
        assert_eq!(pr.cache_write, None);
        // 四个字段都可选 —— 没填的不写出去
        assert_eq!(serde_json::to_string(&pr).unwrap(), json);
    }

    #[test]
    fn synced_pricing_participates_in_the_dirty_hash() {
        // 同步来的费率会改变写进 Claude 的费率行 → 必须进哈希（否则价格变了也不提示重新应用）。
        // 但这也意味着：前端草稿若落后于后台同步，它算出来的哈希就是错的。
        let mut a = sample_config();
        let mut b = sample_config();
        b.providers[0].models[0].pricing_synced = Some(ModelPricing {
            input: Some(0.95),
            ..Default::default()
        });
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
        a.providers[0].models[0].context_limit = Some(262_144);
        assert_ne!(canonical_hash(&a), canonical_hash(&sample_config()));
    }

    #[test]
    fn flatten_carries_effective_pricing_to_the_gateway() {
        let synced = ModelPricing { input: Some(0.95), ..Default::default() };
        let cfg = Config {
            providers: vec![provider(
                "https://a.example.com",
                "k",
                vec![ModelEntry {
                    name: "m".into(),
                    pricing_synced: Some(synced.clone()),
                    ..Default::default()
                }],
                "",
            )],
            ..Default::default()
        };
        assert_eq!(flatten_config(&cfg)[0].pricing, Some(synced));
    }

    #[test]
    fn sync_fields_default_off_the_wire_and_keep_v1_shape() {
        let cfg: Config = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert!(cfg.pricing_auto_sync, "运行时同步默认开，否则「永远是新价」无从谈起");
        assert_eq!(cfg.pricing_synced_at, "");
        let out = serde_json::to_string(&Config::default()).unwrap();
        assert!(!out.contains("pricing_auto_sync"), "{out}");
        assert!(!out.contains("pricing_synced_at"), "{out}");
    }

    // ---- §2.2 换 slot 池后老用户必须重新应用 ----

    #[test]
    fn upgrading_slot_pool_invalidates_applied_hash() {
        // config.json 里不存 slot（flatten 时现算），所以换池子不用迁移配置文件 ——
        // 但 Claude Desktop 里写着的还是旧槽位，不重新应用就用不上新的选择器。
        // 池子代号必须进哈希，否则升级后状态机显示「已生效」，用户永远不会去点应用。
        let cfg = sample_config();
        assert_ne!(
            canonical_hash_with_pool(&cfg, "2.0"),
            canonical_hash_with_pool(&cfg, "2.1")
        );
        assert_eq!(canonical_hash(&cfg), canonical_hash_with_pool(&cfg, SLOT_POOL_VERSION));
    }

    #[test]
    fn last_applied_pool_defaults_empty_and_round_trips() {
        // 老文件没这个键 → 空串，前端据此认出「升级导致的 dirty」
        let cfg: Config = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(cfg.last_applied_pool, "");
        assert!(!serde_json::to_string(&Config::default()).unwrap().contains("last_applied_pool"));

        let mut cfg = sample_config();
        cfg.last_applied_pool = SLOT_POOL_VERSION.to_string();
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains("last_applied_pool"));
        assert_eq!(
            serde_json::from_str::<Config>(&out).unwrap().last_applied_pool,
            SLOT_POOL_VERSION
        );
        // 与 last_applied_hash 一样是 applied 元数据，不参与哈希本身
        assert_eq!(canonical_hash(&cfg), canonical_hash(&sample_config()));
    }

    // ---- port 字段（2026-07-14 新增，默认值不序列化保证 v1 格式兼容） ----

    #[test]
    fn port_defaults_to_5678_and_is_omitted_when_default() {
        let cfg: Config = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(cfg.port, 5678);
        assert_eq!(Config::default().port, 5678);
        let out = serde_json::to_string(&Config::default()).unwrap();
        assert!(!out.contains("port"));
    }

    #[test]
    fn custom_port_round_trips_and_affects_hash() {
        let mut cfg = sample_config();
        cfg.port = 5679;
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains("\"port\":5679"));
        let back: Config = serde_json::from_str(&out).unwrap();
        assert_eq!(back.port, 5679);

        let mut base = sample_config();
        assert_ne!(canonical_hash(&base), canonical_hash(&cfg));
        // 显式 5678 与缺省等价（skip_serializing_if）
        base.port = 5678;
        assert_eq!(canonical_hash(&base), canonical_hash(&sample_config()));
    }

    #[test]
    fn hash_field_round_trips_when_set() {
        let mut cfg = sample_config();
        cfg.last_applied_hash = "abc123".into();
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains("\"last_applied_hash\":\"abc123\""));
        let back: Config = serde_json::from_str(&out).unwrap();
        assert_eq!(back.last_applied_hash, "abc123");
    }
}
