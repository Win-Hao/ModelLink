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

pub const MAX_MODELS: usize = 8;

pub const ANTHROPIC_SLOTS: &[&str] = &[
    "claude-3-opus-latest",
    "claude-3-5-sonnet-latest",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
    "claude-3-5-haiku-latest",
    "claude-3-opus-20240229",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-sonnet-20240620",
];

pub const DEFAULT_PORT: u16 = 5678;

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn is_default_port(p: &u16) -> bool {
    *p == DEFAULT_PORT
}

fn is_false(b: &bool) -> bool {
    !*b
}

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
    /// 2.1-A 新增（§3.3）：兼容模式 —— 未映射的槽位仍回落到第一个模型（v1 的静默行为）。
    /// 默认关：未映射直接 400，用户才知道自己在用什么模型。
    /// 值为 false 时不序列化 —— 老用户文件格式不变。
    #[serde(default, skip_serializing_if = "is_false")]
    pub compat_fallback: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            last_applied_hash: String::new(),
            last_applied_at: String::new(),
            port: DEFAULT_PORT,
            compat_fallback: false,
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
/// port 参与哈希（改端口须重新应用），默认 5678 时不序列化 → 老 hash 不受升级影响。
/// compat_fallback 只影响代理自身的路由行为、不改写 Claude Desktop 的任何键，
/// 故**不**参与哈希（切它不该提示「需重新应用」）。
pub fn canonical_hash(config: &Config) -> String {
    let canon = Config {
        providers: config.providers.clone(),
        port: config.port,
        ..Default::default()
    };
    let json = serde_json::to_string(&canon).unwrap_or_default();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in json.as_bytes() {
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

pub struct FlatEntry {
    pub slot: String,
    pub name: String,
    pub to_1m: String,
    pub url: String,
    pub key: String,
    pub thinking_effort: String,
}

pub fn flatten_config(config: &Config) -> Vec<FlatEntry> {
    let mut result = Vec::new();
    let mut count = 0;
    for provider in &config.providers {
        for m in &provider.models {
            if count < MAX_MODELS && count < ANTHROPIC_SLOTS.len() && !m.name.is_empty() {
                result.push(FlatEntry {
                    slot: ANTHROPIC_SLOTS[count].to_string(),
                    name: m.name.clone(),
                    to_1m: m.to_1m.clone(),
                    url: provider.target_url.clone(),
                    key: provider.api_key.clone(),
                    thinking_effort: provider.thinking_effort.clone(),
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
/// 依赖旧行为的用户可打开 `compat_fallback`（默认关）。
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

    if config.compat_fallback {
        if let Some(e) = flat.into_iter().next() {
            let resolved = if is_1m && !e.to_1m.is_empty() {
                format!("{}[1m]", e.name)
            } else {
                e.name
            };
            eprintln!("  fallback: {} -> {} (兼容模式)", model, resolved);
            return Ok(ResolvedModel {
                model: resolved,
                target_url: e.url,
                api_key: e.key,
                thinking_effort: e.thinking_effort,
            });
        }
    }

    Err(ResolveError::UnmappedSlot(base.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, to_1m: &str) -> ModelEntry {
        ModelEntry { name: name.into(), to_1m: to_1m.into() }
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

    // ---- flatten_config ----

    #[test]
    fn flatten_assigns_slots_in_order_across_providers() {
        let flat = flatten_config(&sample_config());
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].slot, "claude-3-opus-latest");
        assert_eq!(flat[0].name, "model-a1");
        assert_eq!(flat[0].url, "https://a.example.com");
        assert_eq!(flat[0].thinking_effort, "max");
        assert_eq!(flat[1].slot, "claude-3-5-sonnet-latest");
        assert_eq!(flat[1].name, "model-a2");
        assert_eq!(flat[2].slot, "claude-3-sonnet-20240229");
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
        assert_eq!(flat[0].slot, "claude-3-opus-latest");
        assert_eq!(flat[0].name, "real");
    }

    #[test]
    fn flatten_caps_at_eight_slots() {
        let models: Vec<ModelEntry> = (0..12).map(|i| model(&format!("m{i}"), "")).collect();
        let cfg = Config {
            providers: vec![provider("https://a.example.com", "k", models, "")],
            ..Default::default()
        };
        let flat = flatten_config(&cfg);
        assert_eq!(flat.len(), 8);
        assert_eq!(flat[7].slot, "claude-3-5-sonnet-20240620");
        assert_eq!(flat[7].name, "m7");
    }

    // ---- resolve_model ----

    #[test]
    fn resolve_matches_slot_to_provider_model() {
        let cfg = sample_config();
        let r = resolve_model("claude-3-sonnet-20240229", &cfg).unwrap();
        assert_eq!(r.model, "model-b1");
        assert_eq!(r.target_url, "https://b.example.com");
        assert_eq!(r.api_key, "key-b");
        assert_eq!(r.thinking_effort, "off");
    }

    #[test]
    fn resolve_1m_suffix_maps_when_to_1m_set() {
        let cfg = sample_config();
        let r = resolve_model("claude-3-opus-latest[1m]", &cfg).unwrap();
        assert_eq!(r.model, "model-a1[1m]");
    }

    #[test]
    fn resolve_1m_suffix_dropped_when_to_1m_empty() {
        let cfg = sample_config();
        let r = resolve_model("claude-3-5-sonnet-latest[1m]", &cfg).unwrap();
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
            resolve_model("claude-3-haiku-20240307", &cfg),
            Err(ResolveError::UnmappedSlot("claude-3-haiku-20240307".into()))
        );
    }

    #[test]
    fn resolve_empty_config_reports_unmapped_slot() {
        let cfg = Config::default();
        assert_eq!(
            resolve_model("claude-3-opus-latest", &cfg),
            Err(ResolveError::UnmappedSlot("claude-3-opus-latest".into()))
        );
    }

    // ---- 兼容模式（默认关）：还原 v1 的回落行为 ----

    #[test]
    fn compat_mode_restores_v1_fallback_to_first_entry() {
        let mut cfg = sample_config();
        cfg.compat_fallback = true;
        let r = resolve_model("claude-9-nonexistent", &cfg).unwrap();
        assert_eq!(r.model, "model-a1");
        assert_eq!(r.target_url, "https://a.example.com");
        assert_eq!(r.thinking_effort, "max");

        let r = resolve_model("claude-9-nonexistent[1m]", &cfg).unwrap();
        assert_eq!(r.model, "model-a1[1m]");
    }

    #[test]
    fn compat_mode_still_errors_when_nothing_to_fall_back_to() {
        let cfg = Config { compat_fallback: true, ..Default::default() };
        assert_eq!(
            resolve_model("claude-3-opus-latest", &cfg),
            Err(ResolveError::UnmappedSlot("claude-3-opus-latest".into()))
        );
    }

    #[test]
    fn compat_fallback_defaults_off_and_round_trips() {
        // 老文件没有这个键 → 默认关
        let cfg: Config = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert!(!cfg.compat_fallback);
        // 关着时不序列化 —— 老用户文件格式不变
        assert!(!serde_json::to_string(&Config::default()).unwrap().contains("compat_fallback"));
        let mut cfg = sample_config();
        cfg.compat_fallback = true;
        let out = serde_json::to_string(&cfg).unwrap();
        assert!(out.contains("\"compat_fallback\":true"));
        assert!(serde_json::from_str::<Config>(&out).unwrap().compat_fallback);
        // 只影响代理路由，不改 Claude Desktop 的键 → 不参与 dirty 判定
        assert_eq!(canonical_hash(&cfg), canonical_hash(&sample_config()));
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
