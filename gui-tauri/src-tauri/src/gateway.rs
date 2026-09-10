//! Claude Desktop 网关写入 + 重启（平移 v1 claude-model-proxy main.rs:230-640, 833-882）。
//!
//! ⚠️ 兼容红线（docs/gui-rebuild-tauri.md §3 #3）：写入逻辑逐字节等价 ——
//! configLibrary 固定 UUID `a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4`、_meta.json 合并规则、
//! claude_desktop_config.json 的 deploymentMode="3p"、Windows 的 MSIX/LOCALAPPDATA/APPDATA
//! 多路径 fallback 与 developer_settings/config.json 写入。禁止顺手优化。
//!
//! 写入方式一律是「读出已有 JSON → 只改自己那几个键 → 写回」，不清空用户其它字段。

use std::path::PathBuf;

use crate::config::{flatten_config, write_with_retry, Config};
use crate::desktop_version::VersionGate;

pub fn claude_3p_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let home = PathBuf::from(home);

    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/Claude-3p");

    #[cfg(target_os = "windows")]
    let dir = {
        // Prefer Microsoft Store sandbox path if Store version is installed
        let store_dir = (|| -> Option<PathBuf> {
            let localappdata = std::env::var("LOCALAPPDATA").ok()?;
            let packages = PathBuf::from(localappdata).join("Packages");
            let known = packages.join("Claude_pzs8sxrjxfjjc");
            if known.exists() {
                return Some(known.join("LocalCache").join("Roaming").join("Claude-3p"));
            }
            std::fs::read_dir(&packages).ok()?.flatten()
                .find(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
                .map(|e| e.path().join("LocalCache").join("Roaming").join("Claude-3p"))
        })();

        store_dir.unwrap_or_else(|| {
            // Non-Store: check %LOCALAPPDATA%\Claude-3p first (newer installs)
            let localappdata = std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Local"));
            let local_dir = localappdata.join("Claude-3p");
            if local_dir.exists() {
                return local_dir;
            }
            // Then check %APPDATA%\Claude-3p (older installs)
            let appdata = std::env::var("APPDATA").ok().map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming"));
            let roaming_dir = appdata.join("Claude-3p");
            if roaming_dir.exists() {
                return roaming_dir;
            }
            // Neither exists yet — default to APPDATA (Roaming)
            roaming_dir
        })
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let dir = home.join(".config/Claude-3p");

    Some(dir)
}

/// 网关配置里 ModelLink 负责的那几个键 —— 启动自动配置与「应用」按钮的唯一写入点。
/// 调用方先读出已有 JSON，这里只改这几个键，其余字段原样保留（用户在 Claude Desktop
/// 里的其它设置不受影响）。
fn write_gateway_keys(existing: &mut serde_json::Value, port: u16, gate: &VersionGate) {
    existing["coworkEgressAllowedHosts"] = serde_json::json!(["*"]);
    existing["inferenceProvider"] = serde_json::json!("gateway");
    existing["inferenceGatewayBaseUrl"] = serde_json::json!(format!("http://127.0.0.1:{}", port));
    existing["inferenceGatewayApiKey"] = serde_json::json!("proxy");
    // ⚠️ `sso` / `auto` 两个取值 2026-10-07 失效；ModelLink 固定写 bearer，不受影响。
    existing["inferenceGatewayAuthScheme"] = serde_json::json!("bearer");
    // 2.1-A §3.4 补两个一直没写的键：
    // Claude Desktop 里 coworkTabEnabled / isClaudeCodeForDesktopEnabled 都有
    // default:true，唯独 chatTabEnabled 没有 —— 不写就是关的，用户装完看不到 Chat 页。
    // （1.13576.0 起支持；低于该版本的老客户端会忽略未知键。按版本门槛跳过写入
    //   属于 §3.8 版本自适应，排在 E 批。）
    if gate.allows("chatTabEnabled") {
        existing["chatTabEnabled"] = serde_json::json!(true);
    }
    // 免去每次启动都要选部署模式
    existing["disableDeploymentModeChooser"] = serde_json::json!(true);
    // 2.1-C §3.2：流式响应的空闲等待上限，取 schema 允许的最大值。
    // ⚠️ 这个键只在网关往响应里写 keep-alive 时才有意义 —— app.asar 原文：
    // "A response on which nothing at all arrives — no pings — still fails after
    //  about 5 minutes regardless of this key"。所以它必须和 proxy.rs 的心跳合流
    // 配套交付，单写这个键治不了断流。（1.44121.1 起支持，值域 300–1800。）
    // §5.5.5：ModelLink 的 /v1/models 只会返回它自己那几个槽位，而槽位名都是完整 ID，
    // app 本来就会跳过发现流程；显式关掉免得某些路径下白跑一次往返。
    // （实测 Kimi 返回的 4 个 ID 全被名字过滤器删光，纯浪费。）
    existing["modelDiscoveryEnabled"] = serde_json::json!(false);
    if gate.allows("inferenceStreamIdleTimeoutSec") {
        existing["inferenceStreamIdleTimeoutSec"] = serde_json::json!(1800);
    }
}

/// §3.9 网络代理透传。约束照 app 内的说明逐条对齐：
/// - 只接受 `http://` / `https://`，SOCKS 被拒；
/// - 不接受内嵌账号密码（`user:pass@`）；
/// - `localhost` / `127.0.0.1` / `[::1]` / `*.local` 由 app 自动 bypass，
///   所以 ModelLink 自己的 127.0.0.1 网关不受影响；
/// - 代理不通**直接失败，不回落直连**；
/// - 只在启动时读一次，改了要重启 Claude。
///
/// PAC 一旦设了就压过普通代理（app 原文："the PAC file wins and this key is ignored"），
/// 所以两个都填时只写 PAC，免得用户以为普通代理还在生效。
pub fn egress_proxy_url_valid(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() {
        return false;
    }
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return false;
    }
    // 内嵌账号密码：scheme 之后、第一个 / 之前出现 @
    let rest = u.split_once("://").map(|(_, r)| r).unwrap_or("");
    let authority = rest.split('/').next().unwrap_or("");
    !authority.contains('@')
}

fn write_egress_proxy(existing: &mut serde_json::Value, config: &Config, gate: &VersionGate) {
    let remove = |e: &mut serde_json::Value, k: &str| {
        if let Some(o) = e.as_object_mut() {
            o.remove(k);
        }
    };
    if !gate.allows("egressProxyUrl") {
        remove(existing, "egressProxyUrl");
        remove(existing, "egressProxyPacUrl");
        return;
    }
    let pac = config.egress_proxy_pac_url.trim();
    let plain = config.egress_proxy_url.trim();

    if egress_proxy_url_valid(pac) {
        existing["egressProxyPacUrl"] = serde_json::json!(pac);
        // PAC 生效时普通代理会被忽略，别留一个看着像在用的键
        remove(existing, "egressProxyUrl");
    } else if egress_proxy_url_valid(plain) {
        existing["egressProxyUrl"] = serde_json::json!(plain);
        remove(existing, "egressProxyPacUrl");
    } else {
        remove(existing, "egressProxyUrl");
        remove(existing, "egressProxyPacUrl");
    }
}

pub fn ensure_claude_desktop_gateway(port: u16) {
    let claude_dir = match claude_3p_dir() {
        Some(d) => d,
        None => {
            eprintln!("[auto-config] FAIL: cannot determine home directory");
            return;
        }
    };
    eprintln!("[auto-config] Claude-3p dir: {}", claude_dir.display());

    let config_lib = claude_dir.join("configLibrary");
    if let Err(e) = std::fs::create_dir_all(&config_lib) {
        eprintln!("[auto-config] FAIL: cannot create {}: {}", config_lib.display(), e);
        return;
    }

    let our_id = "a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4";
    let meta_path = config_lib.join("_meta.json");
    let mut meta: serde_json::Value = if meta_path.exists() {
        let content = std::fs::read_to_string(&meta_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let applied_id = meta.get("appliedId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_id = if !applied_id.is_empty() && config_lib.join(format!("{}.json", applied_id)).exists() {
        applied_id
    } else {
        our_id.to_string()
    };

    let config_file = config_lib.join(format!("{}.json", target_id));
    let mut existing: serde_json::Value = if config_file.exists() {
        let content = std::fs::read_to_string(&config_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let gate = VersionGate::detect();
    write_gateway_keys(&mut existing, port, &gate);
    if existing.get("inferenceModels").is_none() {
        existing["inferenceModels"] = serde_json::json!([]);
    }

    match serde_json::to_string_pretty(&existing) {
        Ok(data) => match write_with_retry(&config_file, &data) {
            Ok(()) => eprintln!("[auto-config] wrote {}", config_file.display()),
            Err(e) => eprintln!("[auto-config] FAIL write {}: {}", config_file.display(), e),
        },
        Err(e) => { eprintln!("[auto-config] FAIL serialize: {}", e); return; }
    }

    if target_id == our_id {
        meta["appliedId"] = serde_json::json!(our_id);
        let entries = meta.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let already_exists = entries.iter().any(|e| e.get("id").and_then(|i| i.as_str()) == Some(our_id));
        if !already_exists {
            let mut new_entries = entries;
            new_entries.push(serde_json::json!({"id": our_id, "name": "ModelLink"}));
            meta["entries"] = serde_json::json!(new_entries);
        }
    }

    if let Ok(meta_data) = serde_json::to_string_pretty(&meta) {
        let meta_tmp = meta_path.with_extension("json.tmp");
        let _ = std::fs::write(&meta_tmp, &meta_data);
        let _ = std::fs::rename(&meta_tmp, &meta_path);
    }

    fn write_desktop_config(path: &PathBuf) {
        let tmp = path.with_extension("json.tmp");
        let mut json: serde_json::Value = if path.exists() {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };
        json["deploymentMode"] = serde_json::json!("3p");
        if let Ok(out) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(&tmp, &out);
            let _ = std::fs::rename(&tmp, path);
        }
        eprintln!("[auto-config] wrote config: {}", path.display());
    }

    write_desktop_config(&claude_dir.join("claude_desktop_config.json"));

    #[cfg(target_os = "windows")]
    {
        let normal_dir = claude_dir.parent()
            .map(|p| p.join("Claude"))
            .unwrap_or_else(|| {
                let home = std::env::var("APPDATA").unwrap_or_default();
                PathBuf::from(home).join("Claude")
            });
        if let Err(e) = std::fs::create_dir_all(&normal_dir) {
            eprintln!("[auto-config] FAIL create {}: {}", normal_dir.display(), e);
        } else {
            eprintln!("[auto-config] Claude dir: {}", normal_dir.display());
        }

        write_desktop_config(&normal_dir.join("claude_desktop_config.json"));

        let dev_settings = normal_dir.join("developer_settings.json");
        if !dev_settings.exists() {
            match std::fs::write(&dev_settings, r#"{"allowDevTools":true}"#) {
                Ok(()) => eprintln!("[auto-config] wrote {}", dev_settings.display()),
                Err(e) => eprintln!("[auto-config] FAIL write {}: {}", dev_settings.display(), e),
            }
        }

        let normal_config = normal_dir.join("config.json");
        if !normal_config.exists() {
            let _ = std::fs::write(&normal_config, r#"{"locale":"zh-CN","hasTrackedInitialActivation":true}"#);
        }

        let p3_dev = claude_dir.join("developer_settings.json");
        if !p3_dev.exists() {
            let _ = std::fs::write(&p3_dev, r#"{"allowDevTools":true}"#);
        }

        let p3_config = claude_dir.join("config.json");
        if !p3_config.exists() {
            let _ = std::fs::write(&p3_config, r#"{"locale":"zh-CN","hasTrackedInitialActivation":true}"#);
        }

        // Also write to other possible paths as fallback
        let appdata = PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
        let localappdata = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default());
        let fallback_dirs = [
            (appdata.join("Claude-3p"), appdata.join("Claude")),
            (localappdata.join("Claude-3p"), localappdata.join("Claude")),
        ];
        for (fb_3p, fb_claude) in &fallback_dirs {
            if fb_3p == &*claude_dir { continue; }
            let _ = std::fs::create_dir_all(fb_claude);
            let _ = std::fs::create_dir_all(fb_3p);
            write_desktop_config(&fb_claude.join("claude_desktop_config.json"));
            write_desktop_config(&fb_3p.join("claude_desktop_config.json"));
            let dev = fb_claude.join("developer_settings.json");
            if !dev.exists() { let _ = std::fs::write(&dev, r#"{"allowDevTools":true}"#); }
            let dev3p = fb_3p.join("developer_settings.json");
            if !dev3p.exists() { let _ = std::fs::write(&dev3p, r#"{"allowDevTools":true}"#); }
        }
    }


    eprintln!("[auto-config] done.");
}

/// apply 写入的 inferenceModels 条目。
/// 2026-07-14 用户拍板的红线例外：在 v1 的 {name, supports1m} 基础上新增
/// labelOverride=厂商模型名，让新版 Claude（模型列表功能 ≥1.2581.0）的选择器
/// 显示真实模型名；官方语义 "Display-only; name is still what the app sends"，
/// 槽位路由机制不变，老版 Claude 忽略未知字段。
fn inference_models_entries(flat: &[crate::config::FlatEntry]) -> Vec<serde_json::Value> {
    flat.iter()
        .map(|e| {
            let supports_1m = !e.to_1m.is_empty();
            let mut row = serde_json::Map::new();
            row.insert("name".into(), serde_json::json!(e.slot));
            row.insert("supports1m".into(), serde_json::json!(supports_1m));
            row.insert("labelOverride".into(), serde_json::json!(e.name));
            // 2.1-D §3.5：以下三个字段没设就一个都不写。
            // app.asar 里它们各有 show 谓词：prefer1m 要 supports1m，
            // isFamilyDefault 要 anthropicFamilyTier —— 这里照同样的条件把无效组合挡掉。
            if e.prefer_1m && supports_1m {
                row.insert("prefer1m".into(), serde_json::json!(true));
            }
            // app 内是枚举校验（Rn(Ba)，且会先 trim + 小写）。config.json 可以手改，
            // 写进去一个不在表里的值会让整个配置被拒 —— 这里先自己滤一道。
            let tier = e.family_tier.trim().to_ascii_lowercase();
            if crate::config::FAMILY_TIERS.contains(&tier.as_str()) {
                row.insert("anthropicFamilyTier".into(), serde_json::json!(tier));
                if e.family_default {
                    row.insert("isFamilyDefault".into(), serde_json::json!(true));
                }
            }
            serde_json::Value::Object(row)
        })
        .collect()
}

/// `organizationInstructions` 的上限（app.asar：`D().trim().min(1).max(3e3)`）。
/// zod 的 `.max` 按 UTF-16 码元计，故这里也按 UTF-16 截断。
const ORG_INSTRUCTIONS_MAX: usize = 3000;

/// §3.7 写 `organizationInstructions`。
///
/// 内容追加到 Chat / Cowork / Code 的系统提示词（含它们派生的子 agent），
/// app 会告诉模型「这来自管理员，优先于用户个人偏好」—— 是引导不是强制约束。
///
/// ⚠️ schema 是 `.trim().min(1)`：清空后必须**删键**，写空串会被拒，
/// 而一条不合法就可能让整个配置文件失效。
fn write_org_instructions(
    existing: &mut serde_json::Value,
    user_text: &str,
    identity_note: bool,
    slot_map: &[(String, String)],
) {
    let mut parts: Vec<String> = Vec::new();
    if identity_note && !slot_map.is_empty() {
        parts.push(identity_note_text(slot_map));
    }
    let user_text = user_text.trim();
    if !user_text.is_empty() {
        parts.push(user_text.to_string());
    }

    let combined = parts.join("\n\n");
    if combined.is_empty() {
        if let Some(o) = existing.as_object_mut() {
            o.remove("organizationInstructions");
        }
        return;
    }
    existing["organizationInstructions"] = serde_json::json!(clamp_utf16(&combined, ORG_INSTRUCTIONS_MAX));
}

/// §3.7 的兜底文案：把槽位映射摊给模型。
///
/// 模型自己知道「我是 claude-opus-5」（系统提示词里的 `ps()` 会写明 exact model ID），
/// 给出映射它就能反推出真实身份 —— 比笼统说一句「你不是 Claude」有效。
fn identity_note_text(slot_map: &[(String, String)]) -> String {
    let lines: Vec<String> = slot_map.iter().map(|(slot, name)| format!("{slot} = {name}")).collect();
    format!(
        "以下是 ModelLink 本地网关的槽位映射；系统提示词中出现的 Claude 模型名只是路由槽位，不代表你的真实身份：\n{}\n请按你实际对应的真实模型作答。",
        lines.join("\n")
    )
}

/// 按 UTF-16 码元截断（与 zod `.max` 的计数方式一致），并保证不切开字符。
fn clamp_utf16(s: &str, max_units: usize) -> String {
    if s.encode_utf16().count() <= max_units {
        return s.to_string();
    }
    let mut out = String::new();
    let mut units = 0;
    for ch in s.chars() {
        let w = ch.len_utf16();
        if units + w > max_units {
            break;
        }
        out.push(ch);
        units += w;
    }
    out
}

/// 单个价格字段的 schema 值域（app.asar 实测：`O().min(0).max(1e4)`）。
/// 越界的一行会让整张费率表失效，所以宁可钳住。
const PRICE_MAX: f64 = 10_000.0;

/// 费率覆盖行（§3.1）。每个定了价的槽位一行，`name` = 槽位 ID（不是上游真名）。
///
/// ⚠️ 单位固定 USD/百万 token，且 `inputPerMtok` / `outputPerMtok` /
/// `cacheReadPerMtok` / `cacheWritePerMtok` **四个全是必填**（设计文档 §3.1 写的
/// 「全部可选」与 app.asar 里的 schema 不符，以 schema 为准）。因此：
/// - 缓存价没填按 0 计 —— 多数国产服务商不单独收缓存写入费，models.dev 也是不收才不写；
/// - 输入 / 输出价没填就整行不写 —— 那两个数编不得，宁可这个模型不显示费用。
fn inference_model_pricing_entries(
    flat: &[crate::config::FlatEntry],
    usd_rate: f64,
) -> Vec<serde_json::Value> {
    let clamp = |v: f64| v.clamp(0.0, PRICE_MAX);
    let mut out = Vec::new();
    for e in flat {
        let Some(p) = e.pricing.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let p = p.in_usd(usd_rate);
        let (Some(input), Some(output)) = (p.input, p.output) else {
            eprintln!("[pricing] {} 缺输入/输出价，跳过该行", e.slot);
            continue;
        };
        out.push(serde_json::json!({
            "name": e.slot,
            "inputPerMtok": clamp(input),
            "outputPerMtok": clamp(output),
            "cacheReadPerMtok": clamp(p.cache_read.unwrap_or(0.0)),
            "cacheWritePerMtok": clamp(p.cache_write.unwrap_or(0.0)),
        }));
    }
    out
}

/// 写费率两键。⚠️ 一条费率都没填时 `inferenceModelPricingEnabled` 必须为 **false**：
/// 槽位借用的是真实 Claude 型号名，费率注解写明内置 Claude ID 也覆盖其带日期/服务商
/// 的形态 —— 没有覆盖行时引擎会按 Anthropic 官方价估算，那是假账单。宁可不显示费用。
///
/// （`inferenceModelPricing*` 1.37937.0 起支持；按版本门槛跳过写入属于 §3.8，排在 E 批。）
fn write_pricing_keys(
    existing: &mut serde_json::Value,
    flat: &[crate::config::FlatEntry],
    usd_rate: f64,
) {
    let rows = inference_model_pricing_entries(flat, usd_rate);
    existing["inferenceModelPricingEnabled"] = serde_json::json!(!rows.is_empty());
    existing["inferenceModelPricing"] = serde_json::json!(rows);
}

pub fn apply_to_claude_desktop(config: &Config) -> Result<String, String> {
    if config.providers.is_empty() {
        return Err("Please add at least one provider.".to_string());
    }
    for (i, p) in config.providers.iter().enumerate() {
        if p.target_url.is_empty() {
            return Err(format!("Provider {} has no API URL.", i + 1));
        }
        if !p.target_url.starts_with("http://") && !p.target_url.starts_with("https://") {
            return Err(format!("Provider {} URL must start with http:// or https://", i + 1));
        }
        if p.api_key.is_empty() {
            return Err(format!("Provider {} has no API key.", i + 1));
        }
        if p.models.is_empty() {
            return Err(format!("Provider {} has no models.", i + 1));
        }
        for m in &p.models {
            if m.name.is_empty() {
                return Err(format!("Provider {} has a model with empty name.", i + 1));
            }
        }
    }

    // 开着 1M 但已知装不下的模型：不拦，但要说出来。
    // 后果是静默的 —— 引擎照发 1M beta 头，上游按自己的上限截断，用户以为有 1M。
    let mut bad_1m: Vec<String> = Vec::new();
    for p in &config.providers {
        for m in &p.models {
            if m.claims_1m_it_does_not_have() {
                bad_1m.push(format!(
                    "{}（上游仅 {}K）",
                    m.name,
                    m.context_limit.unwrap_or(0) / 1024
                ));
            }
        }
    }

    let claude_dir = claude_3p_dir().ok_or("Cannot find home directory")?;
    let config_lib = claude_dir.join("configLibrary");
    std::fs::create_dir_all(&config_lib).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!("Cannot create directory: {}. Permission denied. Try running as administrator.", config_lib.display())
        } else {
            format!("Cannot create directory: {}", e)
        }
    })?;

    let flat = flatten_config(config);
    let models = inference_models_entries(&flat);

    let meta_path = config_lib.join("_meta.json");
    let mut meta: serde_json::Value = if meta_path.exists() {
        let content = std::fs::read_to_string(&meta_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let applied_id = meta.get("appliedId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let our_id = "a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4";

    let target_id = if !applied_id.is_empty() && config_lib.join(format!("{}.json", applied_id)).exists() {
        applied_id.clone()
    } else {
        our_id.to_string()
    };

    let config_file = config_lib.join(format!("{}.json", target_id));
    let mut existing: serde_json::Value = if config_file.exists() {
        let content = std::fs::read_to_string(&config_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let gate = VersionGate::detect();
    write_gateway_keys(&mut existing, config.port, &gate);
    existing["inferenceModels"] = serde_json::json!(models);
    if gate.allows("inferenceModelPricing") {
        write_pricing_keys(&mut existing, &flat, config.usd_rate);
    }
    write_egress_proxy(&mut existing, config, &gate);
    let slot_map: Vec<(String, String)> =
        flat.iter().map(|e| (e.slot.clone(), e.name.clone())).collect();
    if gate.allows("organizationInstructions") {
        write_org_instructions(
            &mut existing,
            &config.org_instructions,
            config.org_identity_note,
            &slot_map,
        );
    }

    let data = serde_json::to_string_pretty(&existing).map_err(|e| e.to_string())?;
    write_with_retry(&config_file, &data)?;

    if target_id != our_id && !config_lib.join(format!("{}.json", our_id)).exists() {
    } else if target_id == our_id {
        meta["appliedId"] = serde_json::json!(our_id);
        let entries = meta.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let mut new_entries: Vec<serde_json::Value> = entries
            .into_iter()
            .filter(|e| {
                if let Some(id) = e.get("id").and_then(|i| i.as_str()) {
                    id == our_id || config_lib.join(format!("{}.json", id)).exists()
                } else {
                    false
                }
            })
            .collect();
        let already_exists = new_entries.iter().any(|e| e.get("id").and_then(|i| i.as_str()) == Some(our_id));
        if !already_exists {
            new_entries.push(serde_json::json!({"id": our_id, "name": "ModelLink"}));
        }
        meta["entries"] = serde_json::json!(new_entries);
    }

    let meta_data = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    let meta_tmp = meta_path.with_extension("json.tmp");
    std::fs::write(&meta_tmp, &meta_data).map_err(|e| format!("Cannot write _meta.json: {}", e))?;
    std::fs::rename(&meta_tmp, &meta_path).map_err(|e| format!("Cannot update _meta.json: {}", e))?;

    let _ = std::fs::remove_file(config_lib.join("model-proxy.json"));

    let desktop_cfg_path = claude_dir.join("claude_desktop_config.json");
    let desktop_tmp = desktop_cfg_path.with_extension("json.tmp");
    if desktop_cfg_path.exists() {
        let content = std::fs::read_to_string(&desktop_cfg_path).unwrap_or_default();
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if json.get("deploymentMode").and_then(|v| v.as_str()) != Some("3p") {
                let mut json = json;
                json["deploymentMode"] = serde_json::json!("3p");
                if let Ok(out) = serde_json::to_string_pretty(&json) {
                    let _ = std::fs::write(&desktop_tmp, &out);
                    let _ = std::fs::rename(&desktop_tmp, &desktop_cfg_path);
                }
            }
        }
    } else {
        let json = serde_json::json!({"deploymentMode": "3p"});
        if let Ok(out) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(&desktop_tmp, &out);
            let _ = std::fs::rename(&desktop_tmp, &desktop_cfg_path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let normal_dir = claude_dir.parent()
            .map(|p| p.join("Claude"))
            .unwrap_or_else(|| {
                let home = std::env::var("APPDATA").unwrap_or_default();
                PathBuf::from(home).join("Claude")
            });
        let _ = std::fs::create_dir_all(&normal_dir);

        let dev_settings = normal_dir.join("developer_settings.json");
        if !dev_settings.exists() {
            let _ = std::fs::write(&dev_settings, r#"{"allowDevTools":true}"#);
        }

        let normal_config = normal_dir.join("config.json");
        if !normal_config.exists() {
            let _ = std::fs::write(&normal_config, r#"{"locale":"zh-CN","hasTrackedInitialActivation":true}"#);
        }

        let normal_cfg = normal_dir.join("claude_desktop_config.json");
        let normal_tmp = normal_cfg.with_extension("json.tmp");
        if normal_cfg.exists() {
            let content = std::fs::read_to_string(&normal_cfg).unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if json.get("deploymentMode").and_then(|v| v.as_str()) != Some("3p") {
                    let mut json = json;
                    json["deploymentMode"] = serde_json::json!("3p");
                    if let Ok(out) = serde_json::to_string_pretty(&json) {
                        let _ = std::fs::write(&normal_tmp, &out);
                        let _ = std::fs::rename(&normal_tmp, &normal_cfg);
                    }
                }
            }
        } else {
            let json = serde_json::json!({"deploymentMode": "3p"});
            if let Ok(out) = serde_json::to_string_pretty(&json) {
                let _ = std::fs::write(&normal_tmp, &out);
                let _ = std::fs::rename(&normal_tmp, &normal_cfg);
            }
        }

        let p3_dev = claude_dir.join("developer_settings.json");
        if !p3_dev.exists() {
            let _ = std::fs::write(&p3_dev, r#"{"allowDevTools":true}"#);
        }

        let p3_config = claude_dir.join("config.json");
        if !p3_config.exists() {
            let _ = std::fs::write(&p3_config, r#"{"locale":"zh-CN","hasTrackedInitialActivation":true}"#);
        }

        // Also write to other possible paths as fallback
        let appdata = PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
        let localappdata = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default());
        let fallback_dirs = [
            (appdata.join("Claude-3p"), appdata.join("Claude")),
            (localappdata.join("Claude-3p"), localappdata.join("Claude")),
        ];
        for (fb_3p, fb_claude) in &fallback_dirs {
            if fb_3p == &*claude_dir { continue; }
            let _ = std::fs::create_dir_all(fb_claude);
            let _ = std::fs::create_dir_all(fb_3p);
            let deploy_json = serde_json::json!({"deploymentMode": "3p"});
            if let Ok(out) = serde_json::to_string_pretty(&deploy_json) {
                let _ = std::fs::write(fb_claude.join("claude_desktop_config.json"), &out);
                let _ = std::fs::write(fb_3p.join("claude_desktop_config.json"), &out);
            }
            let dev = fb_claude.join("developer_settings.json");
            if !dev.exists() { let _ = std::fs::write(&dev, r#"{"allowDevTools":true}"#); }
        }
    }


    let mut msg = format!("Written to {}", config_file.display());
    if !bad_1m.is_empty() {
        let warn = format!("以下模型开着 1M 但上游装不下，1M 变体不会真的生效：{}", bad_1m.join("、"));
        eprintln!("[apply] WARN: {warn}");
        msg.push('\n');
        msg.push_str(&warn);
    }
    Ok(msg)
}

struct ScopeGuard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) { if let Some(f) = self.0.take() { f(); } }
}
fn scopeguard<F: FnOnce()>(f: F) -> ScopeGuard<F> { ScopeGuard(Some(f)) }

static RESTARTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn restart_claude_desktop() {
    if RESTARTING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _guard = scopeguard(|| RESTARTING.store(false, std::sync::atomic::Ordering::SeqCst));
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("osascript")
                .args(["-e", "tell application \"Claude\" to quit"])
                .output();
            for _ in 0..15 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let out = std::process::Command::new("pgrep")
                    .args(["-x", "Claude"])
                    .output();
                if let Ok(o) = out {
                    if o.stdout.is_empty() {
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = std::process::Command::new("open")
                .args(["-a", "Claude"])
                .output();
            eprintln!("[restart] Claude Desktop restarted.");
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("powershell")
                .args(["-WindowStyle", "Hidden", "-Command", r#"
                    $proc = Get-Process -Name 'Claude' -ErrorAction SilentlyContinue | Select-Object -First 1
                    $path = if ($proc) { $proc.Path } else { $null }
                    Stop-Process -Name 'Claude' -Force -ErrorAction SilentlyContinue
                    Start-Sleep -Seconds 3
                    if ($path -like '*WindowsApps*') {
                        $pkg = Get-AppxPackage | Where-Object { $path.StartsWith($_.InstallLocation) } | Select-Object -First 1
                        if ($pkg) { explorer.exe "shell:AppsFolder\$($pkg.PackageFamilyName)!Claude" }
                    } elseif ($path) {
                        Start-Process $path
                    }
                "#])
                .output();
            eprintln!("[restart] Claude Desktop restarted.");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{flatten_config, ModelEntry, ModelPricing, Provider};

    // ---- §3.8 版本自适应 / §3.9 网络代理 ----

    #[test]
    fn old_desktop_does_not_get_keys_it_cannot_parse() {
        let mut existing = serde_json::json!({});
        write_gateway_keys(&mut existing, 5678, &VersionGate::with_version(Some("1.20000.0")));
        // 1.13576.0 起支持 → 写
        assert_eq!(existing["chatTabEnabled"], true);
        // 1.44121.1 起支持 → 不写
        assert!(existing.get("inferenceStreamIdleTimeoutSec").is_none());
        // 没门槛的键照写
        assert_eq!(existing["inferenceProvider"], "gateway");
    }

    #[test]
    fn ancient_desktop_loses_chat_tab_as_well() {
        let mut existing = serde_json::json!({});
        write_gateway_keys(&mut existing, 5678, &VersionGate::with_version(Some("1.10000.0")));
        assert!(existing.get("chatTabEnabled").is_none());
    }

    #[test]
    fn egress_proxy_accepts_only_what_the_app_accepts() {
        assert!(egress_proxy_url_valid("http://proxy.corp:8080"));
        assert!(egress_proxy_url_valid("https://proxy.corp:8080/path"));
        // SOCKS 被拒
        assert!(!egress_proxy_url_valid("socks5://proxy.corp:1080"));
        // 内嵌账号密码被拒
        assert!(!egress_proxy_url_valid("http://user:pass@proxy.corp:8080"));
        // 路径里的 @ 不算
        assert!(egress_proxy_url_valid("http://proxy.corp/a@b"));
        assert!(!egress_proxy_url_valid(""));
        assert!(!egress_proxy_url_valid("proxy.corp:8080"));
    }

    #[test]
    fn pac_url_wins_over_the_plain_proxy() {
        // app 原文："When egressProxyPacUrl is also set, the PAC file wins and
        // this key is ignored" —— 别留一个看着像在用的键
        let mut existing = serde_json::json!({});
        let cfg = Config {
            egress_proxy_url: "http://proxy.corp:8080".into(),
            egress_proxy_pac_url: "https://corp/proxy.pac".into(),
            ..Default::default()
        };
        write_egress_proxy(&mut existing, &cfg, &VersionGate::with_version(None));
        assert_eq!(existing["egressProxyPacUrl"], "https://corp/proxy.pac");
        assert!(existing.get("egressProxyUrl").is_none());
    }

    #[test]
    fn invalid_or_absent_proxy_clears_both_keys() {
        let mut existing = serde_json::json!({
            "egressProxyUrl": "http://old:8080",
            "egressProxyPacUrl": "https://old/x.pac"
        });
        let cfg = Config {
            egress_proxy_url: "socks5://nope:1080".into(),
            ..Default::default()
        };
        write_egress_proxy(&mut existing, &cfg, &VersionGate::with_version(None));
        assert!(existing.get("egressProxyUrl").is_none());
        assert!(existing.get("egressProxyPacUrl").is_none());
    }

    #[test]
    fn old_desktop_never_gets_proxy_keys() {
        let mut existing = serde_json::json!({});
        let cfg = Config { egress_proxy_url: "http://proxy.corp:8080".into(), ..Default::default() };
        write_egress_proxy(&mut existing, &cfg, &VersionGate::with_version(Some("1.40000.0")));
        assert!(existing.get("egressProxyUrl").is_none());
    }

    // ---- §3.5 模型条目补三个字段 ----

    #[test]
    #[ignore = "改 HOME，需串行运行"]
    fn apply_warns_about_1m_the_upstream_cannot_hold() {
        let tmp = std::env::temp_dir().join(format!("ml-1m-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);

        let cfg = cfg_with(vec![
            ModelEntry {
                name: "Kimi-k2.6".into(),
                to_1m: "auto".into(),
                context_limit: Some(262_144),
                ..Default::default()
            },
            ModelEntry {
                name: "deepseek-v4-pro".into(),
                to_1m: "auto".into(),
                context_limit: Some(1_000_000),
                ..Default::default()
            },
            // 未知上限的不该被点名
            ModelEntry { name: "unknown".into(), to_1m: "auto".into(), ..Default::default() },
        ]);
        let msg = apply_to_claude_desktop(&cfg).unwrap();
        assert!(msg.contains("Kimi-k2.6（上游仅 256K）"), "{msg}");
        assert!(!msg.contains("deepseek-v4-pro"), "{msg}");
        assert!(!msg.contains("unknown"), "{msg}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn model_entries_carry_tier_and_prefer1m_only_when_set() {
        let cfg = cfg_with(vec![
            ModelEntry {
                name: "Kimi-k2.6".into(),
                to_1m: "auto".into(),
                prefer_1m: true,
                family_tier: "opus".into(),
                family_default: true,
                ..Default::default()
            },
            ModelEntry { name: "plain".into(), ..Default::default() },
        ]);
        let e = inference_models_entries(&flatten_config(&cfg));
        assert_eq!(
            e[0],
            serde_json::json!({
                "name": "claude-opus-5",
                "supports1m": true,
                "labelOverride": "Kimi-k2.6",
                "prefer1m": true,
                "anthropicFamilyTier": "opus",
                "isFamilyDefault": true,
            })
        );
        // 没设的字段一个都不写 —— 老版 Claude 见到未知键会忽略，但空值可能触发 schema
        assert_eq!(
            e[1],
            serde_json::json!({
                "name": "claude-sonnet-5",
                "supports1m": false,
                "labelOverride": "plain",
            })
        );
    }

    #[test]
    fn prefer1m_is_dropped_without_supports1m() {
        // app.asar：prefer1m 的 show 谓词是 !!e.supports1m —— 没有 1M 变体时它无意义
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            to_1m: String::new(),
            prefer_1m: true,
            ..Default::default()
        }]);
        let e = inference_models_entries(&flatten_config(&cfg));
        assert!(e[0].get("prefer1m").is_none());
    }

    #[test]
    fn unknown_tier_values_are_dropped_rather_than_written() {
        // config.json 可以手改；写进一个不在枚举里的值会让整个配置被 app 拒掉
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            family_tier: "超级模型".into(),
            family_default: true,
            ..Default::default()
        }]);
        let e = inference_models_entries(&flatten_config(&cfg));
        assert!(e[0].get("anthropicFamilyTier").is_none());
        assert!(e[0].get("isFamilyDefault").is_none());
    }

    #[test]
    fn tier_values_are_trimmed_and_lowercased_like_the_app_does() {
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            family_tier: "  OPUS  ".into(),
            ..Default::default()
        }]);
        let e = inference_models_entries(&flatten_config(&cfg));
        assert_eq!(e[0]["anthropicFamilyTier"], "opus");
    }

    #[test]
    fn family_default_is_dropped_without_a_tier() {
        // app.asar：isFamilyDefault 的 show 谓词是 !!e.anthropicFamilyTier
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            family_default: true,
            ..Default::default()
        }]);
        let e = inference_models_entries(&flatten_config(&cfg));
        assert!(e[0].get("isFamilyDefault").is_none());
        assert!(e[0].get("anthropicFamilyTier").is_none());
    }

    // ---- §3.7 organizationInstructions ----

    #[test]
    fn org_instructions_key_is_removed_when_blank() {
        // app.asar schema：D().trim().min(1).max(3e3) —— 写空串会被拒，
        // 一条不合法就可能让整个配置文件失效，所以必须删键
        let mut existing = serde_json::json!({"organizationInstructions": "旧内容"});
        write_org_instructions(&mut existing, "   \n  ", false, &[]);
        assert!(existing.get("organizationInstructions").is_none());
    }

    #[test]
    fn org_instructions_are_written_verbatim() {
        let mut existing = serde_json::json!({});
        write_org_instructions(&mut existing, "统一用简体中文回答。", false, &[]);
        assert_eq!(existing["organizationInstructions"], "统一用简体中文回答。");
    }

    #[test]
    fn identity_note_prepends_the_slot_mapping() {
        let mut existing = serde_json::json!({});
        let map = [
            ("claude-opus-5".to_string(), "Kimi-k2.6".to_string()),
            ("claude-sonnet-5".to_string(), "glm-5.1".to_string()),
        ];
        write_org_instructions(&mut existing, "统一用简体中文回答。", true, &map);
        let s = existing["organizationInstructions"].as_str().unwrap();
        assert!(s.contains("claude-opus-5 = Kimi-k2.6"), "{s}");
        assert!(s.contains("claude-sonnet-5 = glm-5.1"), "{s}");
        assert!(s.contains("路由槽位"), "{s}");
        assert!(s.ends_with("统一用简体中文回答。"), "用户文本必须原样附在后面: {s}");
    }

    #[test]
    fn identity_note_has_no_stray_indentation() {
        // 这段文字会进模型的系统提示词 —— Rust 多行字符串的续行很容易把缩进带进去
        let map = [("claude-opus-5".to_string(), "Kimi-k2.6".to_string())];
        for line in identity_note_text(&map).lines() {
            assert_eq!(line, line.trim(), "行首/行尾有多余空白: {line:?}");
        }
    }

    #[test]
    fn identity_note_alone_is_enough_to_write_the_key() {
        // 用户没写自定义指令，但开了兜底开关 → 仍然要写
        let mut existing = serde_json::json!({});
        let map = [("claude-opus-5".to_string(), "Kimi-k2.6".to_string())];
        write_org_instructions(&mut existing, "", true, &map);
        assert!(existing["organizationInstructions"].as_str().unwrap().contains("Kimi-k2.6"));
    }

    #[test]
    fn org_instructions_are_clamped_to_the_schema_limit() {
        // 超 3000 会被 schema 拒 → 整个配置失效。按 UTF-16 计数（zod 的 .max 就是这么算的）
        let mut existing = serde_json::json!({});
        let long = "中".repeat(4000);
        write_org_instructions(&mut existing, &long, false, &[]);
        let s = existing["organizationInstructions"].as_str().unwrap();
        assert!(s.encode_utf16().count() <= 3000, "实际 {}", s.encode_utf16().count());
    }

    // ---- §3.1 费率表 ----

    fn priced(name: &str, input: f64, output: f64) -> ModelEntry {
        ModelEntry {
            name: name.into(),
            to_1m: String::new(),
            pricing: Some(ModelPricing {
                input: Some(input),
                output: Some(output),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn cfg_with(models: Vec<ModelEntry>) -> Config {
        Config {
            providers: vec![Provider {
                target_url: "https://a.example.com".into(),
                api_key: "k".into(),
                models,
                thinking_effort: String::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn pricing_entries_are_keyed_by_slot_and_converted_to_usd() {
        let cfg = cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]);
        let entries = inference_model_pricing_entries(&flatten_config(&cfg), cfg.usd_rate);
        assert_eq!(
            entries,
            vec![serde_json::json!({
                // name = 槽位 ID，不是上游真名 —— 引擎按这个匹配用量
                "name": "claude-opus-5",
                "inputPerMtok": 0.5556,
                "outputPerMtok": 2.2222,
                "cacheReadPerMtok": 0.0,
                "cacheWritePerMtok": 0.0,
            })]
        );
    }

    #[test]
    fn pricing_rows_always_carry_all_four_fields() {
        // app.asar 实测：四个价格字段的 schema 是 O().min(0).max(1e4)，**没有 .optional()**
        // ——「四个字段全部可选」是设计文档写错了。缺字段的行会被 schema 拒掉，
        // 而一行不合法很可能让整张费率表（甚至整个配置文件）失效。
        let cfg = cfg_with(vec![priced("m", 4.0, 16.0)]);
        let rows = inference_model_pricing_entries(&flatten_config(&cfg), 7.2);
        let row = rows[0].as_object().unwrap();
        for k in [
            "name",
            "inputPerMtok",
            "outputPerMtok",
            "cacheReadPerMtok",
            "cacheWritePerMtok",
        ] {
            assert!(row.contains_key(k), "缺字段 {k}: {row:?}");
        }
        // 没填的缓存价按 0 计（多数国产服务商不单独收缓存写入费）
        assert_eq!(row["cacheReadPerMtok"], 0.0);
        assert_eq!(row["cacheWritePerMtok"], 0.0);
    }

    #[test]
    fn models_without_input_or_output_price_get_no_row_at_all() {
        // 缓存价可以按 0 兜底，输入/输出价不能编 —— 没有就不写这一行
        let cfg = cfg_with(vec![
            ModelEntry {
                name: "only-cache".into(),
                pricing: Some(ModelPricing { cache_read: Some(0.8), ..Default::default() }),
                ..Default::default()
            },
            ModelEntry {
                name: "only-input".into(),
                pricing: Some(ModelPricing { input: Some(4.0), ..Default::default() }),
                ..Default::default()
            },
        ]);
        assert!(inference_model_pricing_entries(&flatten_config(&cfg), 7.2).is_empty());
    }

    #[test]
    fn prices_are_clamped_into_the_schema_range() {
        // schema 值域 [0, 10000]；越界的一行会让整张表失效，宁可钳住
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            pricing: Some(ModelPricing {
                input: Some(999_999.0),
                output: Some(-5.0),
                cache_read: Some(0.0),
                cache_write: Some(0.0),
                currency: "USD".into(),
            }),
            ..Default::default()
        }]);
        let rows = inference_model_pricing_entries(&flatten_config(&cfg), 7.2);
        assert_eq!(rows[0]["inputPerMtok"], 10000.0);
        assert_eq!(rows[0]["outputPerMtok"], 0.0);
    }

    #[test]
    fn unpriced_models_get_no_row_and_keep_their_slot_position() {
        let cfg = cfg_with(vec![
            ModelEntry { name: "no-price".into(), ..Default::default() },
            ModelEntry {
                name: "full".into(),
                to_1m: String::new(),
                pricing: Some(ModelPricing {
                    input: Some(4.0),
                    output: Some(16.0),
                    cache_read: Some(0.8),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ModelEntry {
                name: "empty-price".into(),
                to_1m: String::new(),
                pricing: Some(ModelPricing::default()),
                ..Default::default()
            },
        ]);
        let entries = inference_model_pricing_entries(&flatten_config(&cfg), 7.2);
        // 只有 full 那条成行；它占的是第 2 个槽位（没定价的模型照样占槽位）
        assert_eq!(
            entries,
            vec![serde_json::json!({
                "name": "claude-sonnet-5",
                "inputPerMtok": 0.5556,
                "outputPerMtok": 2.2222,
                "cacheReadPerMtok": 0.1111,
                "cacheWritePerMtok": 0.0,
            })]
        );
    }

    #[test]
    fn pricing_switch_stays_off_when_nothing_is_priced() {
        // 一条费率都没填时开着 enabled，引擎会拿 Anthropic 官方价估算借来的槽位名 ——
        // 那就是假账单。宁可不显示费用。
        let mut existing = serde_json::json!({});
        let cfg = cfg_with(vec![ModelEntry { name: "m".into(), ..Default::default() }]);
        write_pricing_keys(&mut existing, &flatten_config(&cfg), cfg.usd_rate);
        assert_eq!(existing["inferenceModelPricingEnabled"], false);
        assert_eq!(existing["inferenceModelPricing"], serde_json::json!([]));

        let cfg = cfg_with(vec![priced("m", 4.0, 16.0)]);
        write_pricing_keys(&mut existing, &flatten_config(&cfg), cfg.usd_rate);
        assert_eq!(existing["inferenceModelPricingEnabled"], true);
        assert_eq!(existing["inferenceModelPricing"].as_array().unwrap().len(), 1);
    }

    /// 端到端：整条 apply 写入路径（含费率行）落到磁盘上长什么样。
    /// 改 HOME 会影响整个进程，故串行执行：`cargo test -- --ignored --test-threads=1`。
    #[test]
    #[ignore = "改 HOME，需串行运行"]
    fn apply_writes_a_schema_valid_pricing_table() {
        let tmp = std::env::temp_dir().join(format!("ml-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);

        let mut first = priced("Kimi-k2.6", 4.0, 16.0);
        first.to_1m = "auto".into();
        first.prefer_1m = true;
        first.family_tier = "opus".into();
        first.family_default = true;
        let mut cfg = cfg_with(vec![
            first,
            ModelEntry { name: "no-price".into(), ..Default::default() },
        ]);
        cfg.providers[0].target_url = "https://api.kimi.com/coding/".into();
        cfg.org_instructions = "统一用简体中文回答。".into();
        cfg.org_identity_note = true;
        apply_to_claude_desktop(&cfg).unwrap();

        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                claude_3p_dir()
                    .unwrap()
                    .join("configLibrary/a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4.json"),
            )
            .unwrap(),
        )
        .unwrap();

        // 槽位来自 2.1 新池子
        let models = written["inferenceModels"].as_array().unwrap();
        assert_eq!(models[0]["name"], "claude-opus-5");
        assert_eq!(models[1]["name"], "claude-sonnet-5");

        // 费率：定了价的成行，四字段齐全且都在 schema 值域内；没定价的不出现
        assert_eq!(written["inferenceModelPricingEnabled"], true);
        let rows = written["inferenceModelPricing"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "no-price 那条不该有费率行");
        assert_eq!(rows[0]["name"], "claude-opus-5");
        for k in ["inputPerMtok", "outputPerMtok", "cacheReadPerMtok", "cacheWritePerMtok"] {
            let v = rows[0][k].as_f64().unwrap_or_else(|| panic!("{k} 缺失或不是数字"));
            assert!((0.0..=PRICE_MAX).contains(&v), "{k}={v} 越界");
        }
        assert_eq!(rows[0]["inputPerMtok"], 0.5556);

        // §3.5 模型条目进阶字段：设了的写、没设的不写
        assert_eq!(models[0]["prefer1m"], true);
        assert_eq!(models[0]["anthropicFamilyTier"], "opus");
        assert_eq!(models[0]["isFamilyDefault"], true);
        assert!(models[1].get("prefer1m").is_none());
        assert!(models[1].get("anthropicFamilyTier").is_none());

        // §3.7 组织级指令：兜底说明在前、用户文本原样在后，且不超 3000
        let org = written["organizationInstructions"].as_str().unwrap();
        assert!(org.contains("claude-opus-5 = Kimi-k2.6"), "{org}");
        assert!(org.ends_with("统一用简体中文回答。"), "{org}");
        assert!(org.encode_utf16().count() <= ORG_INSTRUCTIONS_MAX);

        // §3.4 两个键 + 用户其它字段不受影响
        assert_eq!(written["chatTabEnabled"], true);
        assert_eq!(written["disableDeploymentModeChooser"], true);
        assert_eq!(written["inferenceStreamIdleTimeoutSec"], 1800);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// §3.4：这几个键就是 ModelLink 写进网关配置的全部内容 —— 少一个都有用户可见的
    /// 后果（chatTabEnabled 不写 = Chat 页是关的）。改这张表要同步 docs。
    #[test]
    fn gateway_keys_cover_everything_modellink_owns() {
        let mut existing = serde_json::json!({});
        write_gateway_keys(&mut existing, 5678, &VersionGate::with_version(None));
        assert_eq!(
            existing,
            serde_json::json!({
                "coworkEgressAllowedHosts": ["*"],
                "inferenceProvider": "gateway",
                "inferenceGatewayBaseUrl": "http://127.0.0.1:5678",
                "inferenceGatewayApiKey": "proxy",
                "inferenceGatewayAuthScheme": "bearer",
                "chatTabEnabled": true,
                "disableDeploymentModeChooser": true,
                "modelDiscoveryEnabled": false,
                "inferenceStreamIdleTimeoutSec": 1800,
            })
        );
    }

    #[test]
    fn stream_idle_timeout_stays_inside_the_schema_range() {
        // app.asar 实测 schema：Un().int().min(300).max(1800).optional()
        let mut existing = serde_json::json!({});
        write_gateway_keys(&mut existing, 5678, &VersionGate::with_version(None));
        let v = existing["inferenceStreamIdleTimeoutSec"].as_u64().unwrap();
        assert!((300..=1800).contains(&v), "{v} 越界会被 schema 拒掉");
    }

    #[test]
    fn gateway_keys_preserve_other_user_fields() {
        let mut existing = serde_json::json!({
            "someUserSetting": 42,
            "inferenceModels": [{"name": "claude-opus-5"}],
            "chatTabEnabled": false,
        });
        write_gateway_keys(&mut existing, 5679, &VersionGate::with_version(None));
        // 用户其它字段原样保留
        assert_eq!(existing["someUserSetting"], 42);
        assert_eq!(existing["inferenceModels"][0]["name"], "claude-opus-5");
        // 自己负责的键覆盖为新值
        assert_eq!(existing["chatTabEnabled"], true);
        assert_eq!(existing["inferenceGatewayBaseUrl"], "http://127.0.0.1:5679");
    }

    #[test]
    fn inference_models_carry_label_override_and_1m() {
        let cfg = Config {
            providers: vec![Provider {
                target_url: "https://a.example.com".into(),
                api_key: "k".into(),
                models: vec![
                    ModelEntry { name: "Kimi-k2.6".into(), to_1m: "auto".into(), ..Default::default() },
                    ModelEntry { name: "mimo-v2.5-pro".into(), to_1m: "".into(), ..Default::default() },
                ],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let entries = inference_models_entries(&flatten_config(&cfg));
        assert_eq!(
            entries[0],
            serde_json::json!({
                "name": "claude-opus-5",
                "supports1m": true,
                "labelOverride": "Kimi-k2.6"
            })
        );
        assert_eq!(
            entries[1],
            serde_json::json!({
                "name": "claude-sonnet-5",
                "supports1m": false,
                "labelOverride": "mimo-v2.5-pro"
            })
        );
    }
}
