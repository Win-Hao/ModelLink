#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::net::TcpListener;

const PORT: u16 = 5678;
const HTML: &str = include_str!("ui.html");

const MAX_MODELS: usize = 8;

const ANTHROPIC_SLOTS: &[&str] = &[
    "claude-3-opus-latest",
    "claude-3-5-sonnet-latest",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
    "claude-3-5-haiku-latest",
    "claude-3-opus-20240229",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-sonnet-20240620",
];

#[derive(Serialize, Deserialize, Clone, Default)]
struct Config {
    #[serde(default)]
    providers: Vec<Provider>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Provider {
    #[serde(default)]
    target_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    models: Vec<ModelEntry>,
    #[serde(default)]
    thinking_effort: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ModelEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    to_1m: String,
}

#[derive(Serialize, Clone)]
struct LogEntry {
    time: String,
    model: String,
    status: u16,
    #[serde(default)]
    thinking: String,
}

struct AppState {
    config: RwLock<Config>,
    client: Client,
    logs: RwLock<Vec<LogEntry>>,
}

const MAX_LOGS: usize = 100;

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude-model-proxy")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn load_config() -> Config {
    let path = config_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Config::default()
    }
}

fn friendly_write_error(e: &std::io::Error, path: &PathBuf) -> String {
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

fn write_with_retry(path: &PathBuf, data: &str) -> Result<(), String> {
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

fn save_config_file(config: &Config) -> Result<(), String> {
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

struct ResolvedModel {
    model: String,
    target_url: String,
    api_key: String,
    thinking_effort: String,
}

struct FlatEntry {
    slot: String,
    name: String,
    to_1m: String,
    url: String,
    key: String,
    thinking_effort: String,
}

fn flatten_config(config: &Config) -> Vec<FlatEntry> {
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

fn resolve_model(model: &str, config: &Config) -> ResolvedModel {
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
            return ResolvedModel {
                model: resolved,
                target_url: e.url.clone(),
                api_key: e.key.clone(),
                thinking_effort: e.thinking_effort.clone(),
            };
        }
    }
    if let Some(e) = flat.into_iter().next() {
        let resolved = if is_1m && !e.to_1m.is_empty() {
            format!("{}[1m]", e.name)
        } else {
            e.name
        };
        eprintln!("  fallback: {} -> {}", model, resolved);
        ResolvedModel {
            model: resolved,
            target_url: e.url,
            api_key: e.key,
            thinking_effort: e.thinking_effort,
        }
    } else {
        ResolvedModel {
            model: model.to_string(),
            target_url: String::new(),
            api_key: String::new(),
            thinking_effort: String::new(),
        }
    }
}

fn claude_3p_dir() -> Option<PathBuf> {
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

fn ensure_claude_desktop_gateway() {
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

    existing["coworkEgressAllowedHosts"] = serde_json::json!(["*"]);
    existing["inferenceProvider"] = serde_json::json!("gateway");
    existing["inferenceGatewayBaseUrl"] = serde_json::json!(format!("http://127.0.0.1:{}", PORT));
    existing["inferenceGatewayApiKey"] = serde_json::json!("proxy");
    existing["inferenceGatewayAuthScheme"] = serde_json::json!("bearer");
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

fn apply_to_claude_desktop(config: &Config) -> Result<String, String> {
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
    let models: Vec<serde_json::Value> = flat
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.slot,
                "supports1m": !e.to_1m.is_empty()
            })
        })
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

    existing["coworkEgressAllowedHosts"] = serde_json::json!(["*"]);
    existing["inferenceProvider"] = serde_json::json!("gateway");
    existing["inferenceGatewayBaseUrl"] = serde_json::json!(format!("http://127.0.0.1:{}", PORT));
    existing["inferenceGatewayApiKey"] = serde_json::json!("proxy");
    existing["inferenceGatewayAuthScheme"] = serde_json::json!("bearer");
    existing["inferenceModels"] = serde_json::json!(models);

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


    Ok(format!("Written to {}", config_file.display()))
}

// === Handlers ===

async fn ui_handler() -> Html<&'static str> {
    Html(HTML)
}

async fn get_config_handler(State(state): State<Arc<AppState>>) -> Json<Config> {
    let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    Json(config)
}

async fn save_config_handler(
    State(state): State<Arc<AppState>>,
    Json(new_config): Json<Config>,
) -> Json<serde_json::Value> {
    if let Err(e) = save_config_file(&new_config) {
        return Json(serde_json::json!({"ok": false, "message": e}));
    }
    *state.config.write().unwrap_or_else(|e| e.into_inner()) = new_config;
    eprintln!("[config] saved");
    Json(serde_json::json!({"ok": true}))
}

#[derive(Deserialize)]
struct TestRequest {
    target_url: String,
    api_key: String,
    model: String,
}

async fn test_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TestRequest>,
) -> Json<serde_json::Value> {
    if req.target_url.is_empty() || req.api_key.is_empty() || req.model.is_empty() {
        return Json(serde_json::json!({"ok": false, "message": "Please fill in URL, Key, and model name."}));
    }
    if !req.target_url.starts_with("http://") && !req.target_url.starts_with("https://") {
        return Json(serde_json::json!({"ok": false, "message": "URL must start with http:// or https://"}));
    }

    let base = req.target_url.trim_end_matches('/');
    let url = format!("{}/v1/messages", base);
    let body = serde_json::json!({
        "model": req.model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let test_client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| state.client.clone());

    let resp = test_client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", req.api_key))
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_vec(&body).unwrap_or_default())
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            if status == 200 {
                Json(serde_json::json!({"ok": true, "message": format!("Connection successful! (HTTP {})", status)}))
            } else {
                let body = r.text().await.unwrap_or_default();
                let msg = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).map(String::from))
                    .unwrap_or_else(|| format!("HTTP {}", status));
                Json(serde_json::json!({"ok": false, "message": msg}))
            }
        }
        Err(e) => {
            let msg = if e.is_connect() {
                "Cannot connect. Check the URL.".to_string()
            } else if e.is_timeout() {
                "Connection timed out.".to_string()
            } else {
                format!("Error: {}", e)
            };
            Json(serde_json::json!({"ok": false, "message": msg}))
        }
    }
}

async fn apply_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    match apply_to_claude_desktop(&config) {
        Ok(msg) => {
            eprintln!("[apply] {}", msg);
            restart_claude_desktop();
            Json(serde_json::json!({"ok": true, "message": "Applied! Claude Desktop is restarting..."}))
        }
        Err(e) => Json(serde_json::json!({"ok": false, "message": e})),
    }
}

fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let offset_secs: i64 = {
        #[cfg(target_os = "macos")]
        {
            let mut now: libc::time_t = 0;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            unsafe { libc::time(&mut now); libc::localtime_r(&now, &mut tm); }
            tm.tm_gmtoff
        }
        #[cfg(not(target_os = "macos"))]
        { 8 * 3600 }
    };
    let local = (d as i64 + offset_secs) as u64;
    let h = (local % 86400) / 3600;
    let m = (local % 3600) / 60;
    let s = local % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

async fn logs_handler(State(state): State<Arc<AppState>>) -> Json<Vec<LogEntry>> {
    let logs = state.logs.read().unwrap_or_else(|e| e.into_inner()).clone();
    Json(logs)
}

fn autostart_plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("Library/LaunchAgents/com.modellink.plist")
}

fn is_autostart_enabled() -> bool {
    autostart_plist_path().exists()
}

fn set_autostart(enabled: bool) -> Result<(), String> {
    let plist_path = autostart_plist_path();
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_str = exe.display().to_string()
            .replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
            .replace('"', "&quot;").replace('\'', "&apos;");
        let content = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.modellink</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#, exe_str);
        let dir = plist_path.parent().ok_or("Invalid path")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(&plist_path, content).map_err(|e| e.to_string())?;
    } else {
        let _ = std::fs::remove_file(&plist_path);
    }
    Ok(())
}

#[derive(Deserialize)]
struct AutostartRequest { enabled: bool }

async fn autostart_get_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"enabled": is_autostart_enabled()}))
}

async fn autostart_set_handler(Json(req): Json<AutostartRequest>) -> Json<serde_json::Value> {
    match set_autostart(req.enabled) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"ok": false, "message": e})),
    }
}

struct ScopeGuard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) { if let Some(f) = self.0.take() { f(); } }
}
fn scopeguard<F: FnOnce()>(f: F) -> ScopeGuard<F> { ScopeGuard(Some(f)) }

static RESTARTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn restart_claude_desktop() {
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

async fn proxy_fallback(
    State(state): State<Arc<AppState>>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if parts.method == Method::GET && parts.uri.path().contains("/v1/models") {
        let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
        let flat = flatten_config(&config);
        let mut models: Vec<serde_json::Value> = Vec::new();
        for e in &flat {
            models.push(serde_json::json!({
                "id": e.slot,
                "display_name": e.name,
                "created": 0
            }));
            if !e.to_1m.is_empty() {
                models.push(serde_json::json!({
                    "id": format!("{}[1m]", e.slot),
                    "display_name": format!("{} (1M)", e.name),
                    "created": 0
                }));
            }
        }
        let resp = serde_json::json!({ "data": models });
        return Json(resp).into_response();
    }

    if parts.method != Method::POST {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }

    let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();

    let mut data: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let resolved = if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
        let r = resolve_model(model, &config);
        eprintln!("  model: {} -> {} ({})", model, r.model, r.target_url);
        data["model"] = serde_json::json!(r.model);
        r
    } else {
        ResolvedModel {
            model: String::new(),
            target_url: String::new(),
            api_key: String::new(),
            thinking_effort: String::new(),
        }
    };

    // Inject thinking effort from provider config
    let te = &resolved.thinking_effort;
    let thinking_log = if !te.is_empty() && te != "off" {
        data["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": 8192});
        data["output_config"] = serde_json::json!({"effort": te});
        eprintln!("  thinking_effort: {}", te);
        te.clone()
    } else if te == "off" {
        data["thinking"] = serde_json::json!({"type": "disabled"});
        data.as_object_mut().map(|o| o.remove("output_config"));
        eprintln!("  thinking: disabled");
        "off".to_string()
    } else {
        String::new()
    };

    if resolved.target_url.is_empty() {
        eprintln!("  error: no target URL configured for this model");
        return (StatusCode::BAD_GATEWAY, "No API URL configured for this model. Please configure the provider in the proxy app.").into_response();
    }

    let base = resolved.target_url.trim_end_matches('/');
    let url = format!("{}{}", base, parts.uri.path());

    let mut req_builder = state
        .client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", resolved.api_key))
        .header(
            "anthropic-version",
            parts
                .headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("2023-06-01"),
        );

    for h in ["anthropic-beta", "x-api-key", "user-agent"] {
        if let Some(v) = parts.headers.get(h).and_then(|v| v.to_str().ok()) {
            req_builder = req_builder.header(h, v);
        }
    }

    let resp = match req_builder
        .body(serde_json::to_vec(&data).unwrap_or_default())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  proxy error: {}", e);
            return (StatusCode::BAD_GATEWAY, format!("Proxy error: {}", e)).into_response();
        }
    };

    let raw_status = resp.status().as_u16();
    let status = StatusCode::from_u16(raw_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
        let entry = LogEntry {
            time: chrono_now(),
            model: model.to_string(),
            status: raw_status,
            thinking: thinking_log.clone(),
        };
        let mut logs = state.logs.write().unwrap_or_else(|e| e.into_inner());
        logs.push(entry);
        let len = logs.len();
        if len > MAX_LOGS { logs.drain(0..len - MAX_LOGS); }
    }

    let mut headers = HeaderMap::new();
    for (k, v) in resp.headers() {
        if k != "transfer-encoding" && k != "connection" {
            headers.insert(k.clone(), v.clone());
        }
    }

    let stream = resp.bytes_stream();
    let body = Body::from_stream(stream);

    (status, headers, body).into_response()
}

fn start_server() -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;
    rt.block_on(async {
        let config = load_config();
        let _ = save_config_file(&config);

        eprintln!("ModelLink v1.0 — Winhao学AI (抖音搜索同名)");
        eprintln!("本软件完全免费，不可商业化");
        eprintln!("Proxy: http://127.0.0.1:{}", PORT);
        eprintln!("Providers: {}", config.providers.len());

        let state = Arc::new(AppState {
            config: RwLock::new(config),
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?,
            logs: RwLock::new(Vec::new()),
        });

        let app = Router::new()
            .route("/", get(ui_handler))
            .route("/api/config", get(get_config_handler).post(save_config_handler))
            .route("/api/test", post(test_handler))
            .route("/api/apply", post(apply_handler))
            .route("/api/logs", get(logs_handler))
            .route("/api/autostart", get(autostart_get_handler).post(autostart_set_handler))
            .fallback(proxy_fallback)
            .with_state(state);

        let listener = TcpListener::bind(format!("127.0.0.1:{}", PORT))
            .await
            .map_err(|e| format!("Port {} already in use: {}. Please close the other instance first.", PORT, e))?;

        eprintln!("Server ready.");
        axum::serve(listener, app).await.map_err(|e| format!("Server error: {}", e))
    })
}

fn make_tray_icon() -> tray_icon::Icon {
    let size = 22usize;
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f64 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - center;
            let dy = y as f64 - center;
            if (dx * dx + dy * dy).sqrt() <= 8.0 {
                let i = (y * size + x) * 4;
                rgba[i] = 0xD9;
                rgba[i + 1] = 0x77;
                rgba[i + 2] = 0x57;
                rgba[i + 3] = 0xFF;
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, size as u32, size as u32).unwrap()
}

fn main() {
    use tao::{
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;
    use muda::{Menu, MenuItem, MenuEvent, Submenu, PredefinedMenuItem};
    use tray_icon::{TrayIconBuilder, TrayIconEvent};

    ensure_claude_desktop_gateway();

    let server_err = Arc::new(std::sync::Mutex::new(None::<String>));
    let server_err_clone = server_err.clone();
    std::thread::spawn(move || {
        if let Err(e) = start_server() {
            eprintln!("Server failed: {}", e);
            *server_err_clone.lock().unwrap() = Some(e);
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(800));

    if let Some(err) = server_err.lock().unwrap().as_ref() {
        eprintln!("Cannot start: {}", err);
        #[cfg(target_os = "macos")]
        {
            let safe_err = err.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = std::process::Command::new("osascript")
                .args(["-e", &format!("display dialog \"{}\" buttons {{\"OK\"}} with title \"ModelLink\" with icon stop", safe_err)])
                .output();
        }
        std::process::exit(1);
    }

    #[cfg(target_os = "macos")]
    use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};

    let mut event_loop = EventLoopBuilder::new().build();
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let app_menu = Menu::new();
    let edit_submenu = Submenu::new("Edit", true);
    let _ = edit_submenu.append(&PredefinedMenuItem::undo(None));
    let _ = edit_submenu.append(&PredefinedMenuItem::redo(None));
    let _ = edit_submenu.append(&PredefinedMenuItem::separator());
    let _ = edit_submenu.append(&PredefinedMenuItem::cut(None));
    let _ = edit_submenu.append(&PredefinedMenuItem::copy(None));
    let _ = edit_submenu.append(&PredefinedMenuItem::paste(None));
    let _ = edit_submenu.append(&PredefinedMenuItem::select_all(None));
    let _ = app_menu.append(&edit_submenu);

    #[cfg(target_os = "macos")]
    app_menu.init_for_nsapp();

    let window = WindowBuilder::new()
        .with_title("ModelLink — Winhao学AI (免费软件，不可商业化)")
        .with_inner_size(tao::dpi::LogicalSize::new(860.0, 760.0))
        .build(&event_loop)
        .expect("Failed to create window");

    let _webview = WebViewBuilder::new()
        .with_url(format!("http://127.0.0.1:{}", PORT))
        .build(&window)
        .expect("Failed to create webview");

    let tray_menu = Menu::new();
    let show_item = MenuItem::new("显示窗口", true, None);
    let quit_item = MenuItem::new("退出", true, None);
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();
    tray_menu.append(&show_item).unwrap();
    tray_menu.append(&quit_item).unwrap();

    let _tray = TrayIconBuilder::new()
        .with_icon(make_tray_icon())
        .with_menu(Box::new(tray_menu))
        .with_menu_on_left_click(false)
        .with_tooltip("ModelLink - Winhao学AI")
        .build()
        .expect("Failed to create tray icon");

    let menu_rx = MenuEvent::receiver().clone();
    let tray_rx = TrayIconEvent::receiver().clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Ok(ev) = menu_rx.try_recv() {
            if ev.id == show_id {
                window.set_visible(true);
                window.set_focus();
            } else if ev.id == quit_id {
                *control_flow = ControlFlow::Exit;
            }
        }

        if let Ok(event) = tray_rx.try_recv() {
            if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. } = event {
                window.set_visible(true);
                window.set_focus();
            }
        }

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                window.set_visible(false);
            }
            _ => {}
        }
    });
}
