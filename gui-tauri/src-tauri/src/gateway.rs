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
fn write_gateway_keys(existing: &mut serde_json::Value, port: u16) {
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
    existing["chatTabEnabled"] = serde_json::json!(true);
    // 免去每次启动都要选部署模式
    existing["disableDeploymentModeChooser"] = serde_json::json!(true);
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

    write_gateway_keys(&mut existing, port);
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
                "labelOverride": e.name
            })
        })
        .collect()
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

    write_gateway_keys(&mut existing, config.port);
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
    use crate::config::{flatten_config, ModelEntry, Provider};

    /// §3.4：这七个键就是 ModelLink 写进网关配置的全部内容 —— 少一个都有用户可见的
    /// 后果（chatTabEnabled 不写 = Chat 页是关的）。改这张表要同步 docs。
    #[test]
    fn gateway_keys_cover_the_seven_modellink_owns() {
        let mut existing = serde_json::json!({});
        write_gateway_keys(&mut existing, 5678);
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
            })
        );
    }

    #[test]
    fn gateway_keys_preserve_other_user_fields() {
        let mut existing = serde_json::json!({
            "someUserSetting": 42,
            "inferenceModels": [{"name": "claude-3-opus-latest"}],
            "chatTabEnabled": false,
        });
        write_gateway_keys(&mut existing, 5679);
        // 用户其它字段原样保留
        assert_eq!(existing["someUserSetting"], 42);
        assert_eq!(existing["inferenceModels"][0]["name"], "claude-3-opus-latest");
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
                    ModelEntry { name: "Kimi-k2.6".into(), to_1m: "auto".into() },
                    ModelEntry { name: "mimo-v2.5-pro".into(), to_1m: "".into() },
                ],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let entries = inference_models_entries(&flatten_config(&cfg));
        assert_eq!(
            entries[0],
            serde_json::json!({
                "name": "claude-3-opus-latest",
                "supports1m": true,
                "labelOverride": "Kimi-k2.6"
            })
        );
        assert_eq!(
            entries[1],
            serde_json::json!({
                "name": "claude-3-5-sonnet-latest",
                "supports1m": false,
                "labelOverride": "mimo-v2.5-pro"
            })
        );
    }
}
