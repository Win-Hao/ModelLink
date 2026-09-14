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
            serde_json::json!({
                "name": e.slot,
                "supports1m": !e.to_1m.is_empty(),
                "labelOverride": e.name,
            })
        })
        .collect()
}

/// `organizationInstructions` 的上限（app.asar：`D().trim().min(1).max(3e3)`）。
/// zod 的 `.max` 按 UTF-16 码元计，故这里也按 UTF-16 截断。
const ORG_INSTRUCTIONS_MAX: usize = 3000;

/// §3.7 写 `organizationInstructions`：把槽位映射摊给模型。
///
/// 内容追加到 Chat / Cowork / Code 的系统提示词（含它们派生的子 agent），
/// app 会告诉模型「这来自管理员，优先于用户个人偏好」—— 是引导不是强制约束。
///
/// **不做成设置项**：Chat 模式跑的是 Claude Code 引擎，系统提示词第 [1] 块是
/// 第二人称角色断言（"You are a Claude agent…"），实测会让部分国产模型自称
/// Claude。用户没有理由关掉它，那就不该问。
///
/// 文案与转发时的「换成这一次的真实模型」见 `identity` 模块。
///
/// ⚠️ schema 是 `.trim().min(1)`：没有内容时必须**删键**，写空串会被拒，
/// 而一条不合法就可能让整个配置文件失效。
fn write_org_instructions(existing: &mut serde_json::Value, slot_map: &[(String, String)]) {
    if slot_map.is_empty() {
        if let Some(o) = existing.as_object_mut() {
            o.remove("organizationInstructions");
        }
        return;
    }
    let text = clamp_utf16(&crate::identity::static_note(slot_map), ORG_INSTRUCTIONS_MAX);
    existing["organizationInstructions"] = serde_json::json!(text);
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
fn inference_model_pricing_entries(flat: &[crate::config::FlatEntry]) -> Vec<serde_json::Value> {
    let clamp = |v: f64| v.clamp(0.0, PRICE_MAX);
    let mut out = Vec::new();
    for e in flat {
        let Some(p) = e.pricing.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let p = p.clone();
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
fn write_pricing_keys(existing: &mut serde_json::Value, flat: &[crate::config::FlatEntry]) {
    let rows = inference_model_pricing_entries(flat);
    // ⚠️ **所有**已路由模型都有价才开。只要有一个没价，那一条就会退回按 Anthropic
    // 官方价估算（费率注解：内置 Claude ID 也覆盖其带日期/服务商的形态）——
    // 一份半真半假的账单比不显示费用更糟，而且现在没有手填入口可以补救。
    let all_priced = !flat.is_empty() && rows.len() == flat.len();
    existing["inferenceModelPricingEnabled"] = serde_json::json!(all_priced);
    existing["inferenceModelPricing"] = serde_json::json!(if all_priced { rows } else { Vec::new() });
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

    // 开着 1M 但已知装不下的模型：不拦，但要说出来。
    // 后果是静默的 —— 引擎照发 1M beta 头，上游按自己的上限截断，用户以为有 1M。
    // 只看 flat：超出 MAX_MODELS 的模型压根不会写进 Claude，警告它没有意义。
    let bad_1m: Vec<String> = flat
        .iter()
        .filter(|e| e.claims_1m_it_does_not_have())
        .map(|e| format!("{}（上游仅 {}K）", e.name, e.context_limit.unwrap_or(0) / 1024))
        .collect();

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
        write_pricing_keys(&mut existing, &flat);
    }
    let slot_map: Vec<(String, String)> =
        flat.iter().map(|e| (e.slot.clone(), e.name.clone())).collect();
    if gate.allows("organizationInstructions") {
        write_org_instructions(&mut existing, &slot_map);
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

/// Claude Desktop 眼下实际在用的那份网关配置里，和 ModelLink 有关的部分。
///
/// 概览页据此判断每个槽位「已生效 / 未应用」：比的是 Claude 那边真正写着的东西，
/// 而不是 ModelLink 自己记得写过什么 —— 配置被 Claude 或别的工具改过时也照实显示。
#[derive(serde::Serialize, Default, Debug, PartialEq)]
pub struct AppliedState {
    /// 找到并读懂了那份配置文件
    pub found: bool,
    /// `inferenceProvider`（ModelLink 写的是 "gateway"）
    pub provider: String,
    /// `inferenceGatewayBaseUrl`
    pub gateway_url: String,
    pub models: Vec<AppliedModel>,
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct AppliedModel {
    pub slot: String,
    /// `labelOverride`；老版本写入的条目没有这个字段，为空
    pub label: String,
    pub supports_1m: bool,
}

/// 从网关配置 JSON 里摘出 [`AppliedState`]。字段缺失或类型不对一律按「没有」处理。
fn parse_applied_state(json: &serde_json::Value) -> AppliedState {
    let text = |key: &str| json.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let models = json
        .get("inferenceModels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let slot = m.get("name")?.as_str()?.to_string();
                    Some(AppliedModel {
                        slot,
                        label: m.get("labelOverride").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        supports_1m: m.get("supports1m").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AppliedState { found: true, provider: text("inferenceProvider"), gateway_url: text("inferenceGatewayBaseUrl"), models }
}

/// Claude Desktop 正在用的那份网关配置文件（不保证存在）。选文件的规则与写入时一致：
/// `_meta.json` 里 `appliedId` 指向的文件存在就用它，否则是 ModelLink 自己那份。
pub fn applied_config_file() -> Option<PathBuf> {
    let config_lib = claude_3p_dir()?.join("configLibrary");
    let our_id = "a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4";
    let applied_id = std::fs::read_to_string(config_lib.join("_meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|m| m.get("appliedId").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_default();
    let target_id = if !applied_id.is_empty() && config_lib.join(format!("{}.json", applied_id)).exists() {
        applied_id
    } else {
        our_id.to_string()
    };
    Some(config_lib.join(format!("{}.json", target_id)))
}

/// 读回 Claude Desktop 正在用的网关配置（只读）。
pub fn read_applied_state() -> AppliedState {
    read_applied_json().map(|json| parse_applied_state(&json)).unwrap_or_default()
}

fn read_applied_json() -> Option<serde_json::Value> {
    applied_config_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
}

/// 「要不要应用」里逐槽位比对（前端比 [`AppliedState::models`]）管不到的部分。
///
/// 判断依据是 Claude 那边真正写着的东西，不是配置哈希：只改了密钥 / 地址 / 默认档时
/// 代理当场就用上了，这几项都不会变 —— 不该为此让用户重启一次 Claude。
#[derive(serde::Serialize, Default, Debug, PartialEq)]
pub struct PendingApply {
    /// 找到并读懂了 Claude 正在用的那份配置
    pub found: bool,
    /// 网关没指向这个代理（没接到 ModelLink，或端口对不上）
    pub gateway: bool,
    /// 端口在上次应用之后换过。`set_port` 会立刻改写配置文件里的地址，
    /// 所以文件对得上也不代表 Claude 用上了 —— 它要重启才读新地址。
    /// `None` = 不知道（2.2 之前应用的，或从没应用过）
    pub port_changed: Option<bool>,
    /// 写进 Claude 的费率表和按现在的配置该写的不一样（同步来了新价、换了模型）
    pub pricing: bool,
}

fn pending_against(config: &Config, file: &serde_json::Value, gate: &VersionGate) -> PendingApply {
    let url = format!("http://127.0.0.1:{}", config.port);
    let gateway = file.get("inferenceProvider").and_then(|v| v.as_str()) != Some("gateway")
        || file.get("inferenceGatewayBaseUrl").and_then(|v| v.as_str()) != Some(url.as_str());
    // 用写入时的同一个函数生成「该写成什么样」，再和文件逐值比 —— 不在这里另写一套规则
    let pricing = gate.allows("inferenceModelPricing") && {
        let mut want = serde_json::json!({});
        write_pricing_keys(&mut want, &flatten_config(config));
        want.get("inferenceModelPricingEnabled") != file.get("inferenceModelPricingEnabled")
            || want.get("inferenceModelPricing") != file.get("inferenceModelPricing")
    };
    PendingApply {
        found: true,
        gateway,
        port_changed: config.last_applied_port.map(|p| p != config.port),
        pricing,
    }
}

/// 见 [`PendingApply`]。没找到配置文件时 `found=false`，其余按「还没接上」处理。
pub fn read_pending_apply(config: &Config) -> PendingApply {
    match read_applied_json() {
        Some(json) => pending_against(config, &json, &VersionGate::detect()),
        None => PendingApply {
            found: false,
            gateway: true,
            port_changed: config.last_applied_port.map(|p| p != config.port),
            pricing: false,
        },
    }
}

struct ScopeGuard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) { if let Some(f) = self.0.take() { f(); } }
}
fn scopeguard<F: FnOnce()>(f: F) -> ScopeGuard<F> { ScopeGuard(Some(f)) }

static RESTARTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 系统账户里登记的主目录（不看 HOME 环境变量）。
#[cfg(target_os = "macos")]
fn login_home() -> Option<String> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(libc::getuid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result)
    };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }.to_str().ok().map(String::from)
}

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
            let mut open = std::process::Command::new("open");
            open.args(["-a", "Claude"]);
            // Claude 会继承这里的环境变量。平时 ModelLink 的 HOME 就是用户主目录，
            // 但用临时 HOME 跑 ModelLink 时（界面预览、回归）继承下去的 Claude 找不到钥匙串，
            // 会弹「找不到用于储存 "Claude Key" 的钥匙串」，读的也是临时目录里的配置。
            if let Some(home) = login_home() {
                open.env("HOME", home);
            }
            let _ = open.output();
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

        // 超出 MAX_MODELS 的模型不会写进 Claude，警告它没有意义
        let mut many = cfg.clone();
        for i in 0..30 {
            many.providers[0].models.push(ModelEntry {
                name: format!("overflow-{i}"),
                to_1m: "auto".into(),
                context_limit: Some(100_000),
                ..Default::default()
            });
        }
        let msg = apply_to_claude_desktop(&many).unwrap();
        assert!(msg.contains("overflow-0"), "槽位内的应当报: {msg}");
        assert!(!msg.contains("overflow-25"), "超出 20 槽位的不该报: {msg}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ---- §3.7 organizationInstructions ----

    #[test]
    fn identity_note_prepends_the_slot_mapping() {
        let mut existing = serde_json::json!({});
        let map = [
            ("claude-opus-5".to_string(), "Kimi-k2.6".to_string()),
            ("claude-sonnet-5".to_string(), "glm-5.1".to_string()),
        ];
        write_org_instructions(&mut existing, &map);
        let s = existing["organizationInstructions"].as_str().unwrap();
        assert!(s.contains("claude-opus-5 = Kimi-k2.6"), "{s}");
        assert!(s.contains("claude-sonnet-5 = glm-5.1"), "{s}");
    }

    // ---- §3.1 费率表 ----

    fn priced(name: &str, input: f64, output: f64) -> ModelEntry {
        ModelEntry {
            name: name.into(),
            to_1m: String::new(),
            pricing_synced: Some(ModelPricing {
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
    fn pricing_entries_are_keyed_by_slot_and_written_verbatim() {
        let cfg = cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]);
        let entries = inference_model_pricing_entries(&flatten_config(&cfg));
        assert_eq!(
            entries,
            vec![serde_json::json!({
                // name = 槽位 ID，不是上游真名 —— 引擎按这个匹配用量
                "name": "claude-opus-5",
                "inputPerMtok": 4.0,
                "outputPerMtok": 16.0,
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
        let rows = inference_model_pricing_entries(&flatten_config(&cfg));
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
                pricing_synced: Some(ModelPricing { cache_read: Some(0.8), ..Default::default() }),
                ..Default::default()
            },
            ModelEntry {
                name: "only-input".into(),
                pricing_synced: Some(ModelPricing { input: Some(4.0), ..Default::default() }),
                ..Default::default()
            },
        ]);
        assert!(inference_model_pricing_entries(&flatten_config(&cfg)).is_empty());
    }

    #[test]
    fn prices_are_clamped_into_the_schema_range() {
        // schema 值域 [0, 10000]；越界的一行会让整张表失效，宁可钳住
        let cfg = cfg_with(vec![ModelEntry {
            name: "m".into(),
            pricing_synced: Some(ModelPricing {
                input: Some(999_999.0),
                output: Some(-5.0),
                cache_read: Some(0.0),
                cache_write: Some(0.0)
            }),
            ..Default::default()
        }]);
        let rows = inference_model_pricing_entries(&flatten_config(&cfg));
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
                pricing_synced: Some(ModelPricing {
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
                pricing_synced: Some(ModelPricing::default()),
                ..Default::default()
            },
        ]);
        let entries = inference_model_pricing_entries(&flatten_config(&cfg));
        // 只有 full 那条成行；它占的是第 2 个槽位（没定价的模型照样占槽位）
        assert_eq!(
            entries,
            vec![serde_json::json!({
                "name": "claude-sonnet-5",
                "inputPerMtok": 4.0,
                "outputPerMtok": 16.0,
                "cacheReadPerMtok": 0.8,
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
        write_pricing_keys(&mut existing, &flatten_config(&cfg));
        assert_eq!(existing["inferenceModelPricingEnabled"], false);
        assert_eq!(existing["inferenceModelPricing"], serde_json::json!([]));

        let cfg = cfg_with(vec![priced("m", 4.0, 16.0)]);
        write_pricing_keys(&mut existing, &flatten_config(&cfg));
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
        let mut cfg = cfg_with(vec![
            first,
            ModelEntry { name: "no-price".into(), ..Default::default() },
        ]);
        cfg.providers[0].target_url = "https://api.kimi.com/coding/".into();
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

        // 有一个模型没价 → 整张表不开（半真半假的账单比不显示更糟）
        assert_eq!(written["inferenceModelPricingEnabled"], false);
        assert!(written["inferenceModelPricing"].as_array().unwrap().is_empty());

        // 全部都有价时才写出完整的四字段行
        let mut all = cfg.clone();
        all.providers[0].models[1].pricing_synced = Some(ModelPricing {
            input: Some(1.0),
            output: Some(2.0),
            ..Default::default()
        });
        apply_to_claude_desktop(&all).unwrap();
        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                claude_3p_dir()
                    .unwrap()
                    .join("configLibrary/a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(written["inferenceModelPricingEnabled"], true);
        let rows = written["inferenceModelPricing"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "claude-opus-5");
        for k in ["inputPerMtok", "outputPerMtok", "cacheReadPerMtok", "cacheWritePerMtok"] {
            let v = rows[0][k].as_f64().unwrap_or_else(|| panic!("{k} 缺失或不是数字"));
            assert!((0.0..=PRICE_MAX).contains(&v), "{k}={v} 越界");
        }
        assert_eq!(rows[0]["inputPerMtok"], 4.0);

        // §3.7 组织级指令：槽位映射说明，且不超 3000
        let org = written["organizationInstructions"].as_str().unwrap();
        assert!(org.contains("claude-opus-5 = Kimi-k2.6"), "{org}");
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

    /// 重启 Claude 时要给它真实主目录 —— 哪怕 ModelLink 自己是用临时 HOME 跑的。
    #[cfg(target_os = "macos")]
    #[test]
    fn login_home_is_a_real_home_directory() {
        let home = login_home().expect("应当能从账户信息里拿到主目录");
        assert!(home.starts_with('/') && !home.starts_with("/tmp"), "{home}");
        assert!(std::path::Path::new(&home).is_dir());
    }

    /// 概览页的「已生效」读的就是这份东西 —— 写进去什么，读回来就得是什么。
    #[test]
    fn applied_state_reads_back_what_apply_writes() {
        let cfg = cfg_with(vec![
            ModelEntry { name: "Kimi-k2.6".into(), to_1m: "auto".into(), ..Default::default() },
            ModelEntry { name: "mimo-v2.5-pro".into(), to_1m: "".into(), ..Default::default() },
        ]);
        let mut written = serde_json::json!({ "someUserSetting": 1 });
        write_gateway_keys(&mut written, 5679, &VersionGate::with_version(None));
        written["inferenceModels"] = serde_json::json!(inference_models_entries(&flatten_config(&cfg)));

        assert_eq!(
            parse_applied_state(&written),
            AppliedState {
                found: true,
                provider: "gateway".into(),
                gateway_url: "http://127.0.0.1:5679".into(),
                models: vec![
                    AppliedModel { slot: "claude-opus-5".into(), label: "Kimi-k2.6".into(), supports_1m: true },
                    AppliedModel { slot: "claude-sonnet-5".into(), label: "mimo-v2.5-pro".into(), supports_1m: false },
                ],
            }
        );
    }

    /// 模拟一次「应用」写进文件的内容（与 apply_to_claude_desktop 同一组函数）。
    fn written_by_apply(cfg: &Config, gate: &VersionGate) -> serde_json::Value {
        let flat = flatten_config(cfg);
        let mut file = serde_json::json!({ "someUserSetting": 1 });
        write_gateway_keys(&mut file, cfg.port, gate);
        file["inferenceModels"] = serde_json::json!(inference_models_entries(&flat));
        if gate.allows("inferenceModelPricing") {
            write_pricing_keys(&mut file, &flat);
        }
        file
    }

    fn applied(cfg: Config) -> Config {
        Config { last_applied_port: Some(cfg.port), ..cfg }
    }

    /// 刚应用完：什么都不欠。
    #[test]
    fn nothing_is_pending_right_after_apply() {
        let gate = VersionGate::with_version(None);
        let cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0), priced("k3", 0.6, 2.5)]));
        let file = written_by_apply(&cfg, &gate);
        assert_eq!(
            pending_against(&cfg, &file, &gate),
            PendingApply { found: true, gateway: false, port_changed: Some(false), pricing: false }
        );
    }

    /// 只改密钥 / 地址 / 默认档：代理当场用上，不该要求重启 Claude（哈希会变，但这里不能报）。
    #[test]
    fn key_url_and_effort_edits_do_not_need_apply() {
        let gate = VersionGate::with_version(None);
        let cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]));
        let file = written_by_apply(&cfg, &gate);

        let mut edited = cfg.clone();
        edited.providers[0].api_key = "a-new-key".into();
        edited.providers[0].target_url = "https://b.example.com".into();
        edited.providers[0].thinking_effort = "max".into();
        assert_ne!(crate::config::canonical_hash(&cfg), crate::config::canonical_hash(&edited));
        assert_eq!(
            pending_against(&edited, &file, &gate),
            PendingApply { found: true, gateway: false, port_changed: Some(false), pricing: false }
        );
    }

    /// 同步来了新价：Claude 里的费用还按旧价算，要应用。
    #[test]
    fn new_prices_need_apply() {
        let gate = VersionGate::with_version(None);
        let cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]));
        let file = written_by_apply(&cfg, &gate);
        let repriced = applied(cfg_with(vec![priced("Kimi-k2.6", 3.0, 16.0)]));
        assert!(pending_against(&repriced, &file, &gate).pricing);
    }

    /// 从「有模型没价（费用关着）」变成「全都有价」也要应用 —— 开关本身变了。
    #[test]
    fn pricing_switch_flip_needs_apply() {
        let gate = VersionGate::with_version(None);
        let unpriced = applied(cfg_with(vec![ModelEntry { name: "k3".into(), ..Default::default() }]));
        let file = written_by_apply(&unpriced, &gate);
        let priced_now = applied(cfg_with(vec![priced("k3", 0.6, 2.5)]));
        assert!(pending_against(&priced_now, &file, &gate).pricing);
    }

    /// 端口热切换已经把新地址写进文件，但 Claude 没重启就还连着旧端口 —— 文件对得上也要应用。
    #[test]
    fn port_switch_needs_apply_even_though_the_file_already_has_the_new_url() {
        let gate = VersionGate::with_version(None);
        let mut cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]));
        cfg.port = 5679; // set_port 之后：config.port 变了，last_applied_port 还是 5678
        let mut file = written_by_apply(&cfg, &gate);
        write_gateway_keys(&mut file, 5679, &gate); // set_port 里的 ensure_claude_desktop_gateway
        let p = pending_against(&cfg, &file, &gate);
        assert!(!p.gateway);
        assert_eq!(p.port_changed, Some(true));
    }

    /// Claude 配置被别的工具改走了：网关不指向这里。
    #[test]
    fn gateway_pointing_elsewhere_needs_apply() {
        let gate = VersionGate::with_version(None);
        let cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]));
        let mut file = written_by_apply(&cfg, &gate);
        file["inferenceGatewayBaseUrl"] = serde_json::json!("http://127.0.0.1:9999");
        assert!(pending_against(&cfg, &file, &gate).gateway);
        file["inferenceGatewayBaseUrl"] = serde_json::json!("http://127.0.0.1:5678");
        file["inferenceProvider"] = serde_json::json!("anthropic");
        assert!(pending_against(&cfg, &file, &gate).gateway);
    }

    /// 2.2 之前应用的：不知道当时的端口，交给前端退回按哈希判断。
    #[test]
    fn port_change_is_unknown_before_the_first_22_apply() {
        let gate = VersionGate::with_version(None);
        let cfg = cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]);
        let file = written_by_apply(&cfg, &gate);
        assert_eq!(pending_against(&cfg, &file, &gate).port_changed, None);
    }

    /// 老版 Claude 不认费率键：ModelLink 也不写，自然不能因为费率要求应用。
    #[test]
    fn old_desktop_without_pricing_never_reports_pricing() {
        let old = VersionGate::with_version(Some("1.30000.0"));
        let cfg = applied(cfg_with(vec![priced("Kimi-k2.6", 4.0, 16.0)]));
        let file = written_by_apply(&cfg, &old);
        assert!(!pending_against(&cfg, &file, &old).pricing);
    }

    /// 老版本写的条目没有 labelOverride / supports1m；坏条目（没有 name）直接跳过，不连累整份。
    #[test]
    fn applied_state_tolerates_old_and_broken_entries() {
        let json = serde_json::json!({
            "inferenceModels": [
                { "name": "claude-3-opus-latest", "supports1m": true },
                { "labelOverride": "no-slot" },
                { "name": "claude-3-5-sonnet-latest" },
            ]
        });
        let st = parse_applied_state(&json);
        assert!(st.found);
        assert_eq!(st.provider, "");
        assert_eq!(
            st.models,
            vec![
                AppliedModel { slot: "claude-3-opus-latest".into(), label: "".into(), supports_1m: true },
                AppliedModel { slot: "claude-3-5-sonnet-latest".into(), label: "".into(), supports_1m: false },
            ]
        );
    }
}
