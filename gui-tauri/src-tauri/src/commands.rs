//! Tauri 命令层（IPC 契约见 docs/gui-rebuild-tauri.md §5）。
//! test_provider / apply 的校验与话术自 v1 /api/* handlers 平移，行为等价。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::config::{canonical_hash, save_config_file, Config};
use crate::gateway;
use crate::models_dev;
use crate::proxy::{LogEntry, ProxyState, TodayStats};

/// GUI 自身版本号（供前端设置页显示）。
#[tauri::command]
pub fn gui_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn get_config(state: State<'_, Arc<ProxyState>>) -> Config {
    state.config.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 保存配置。**返回后端合并后的那份** —— 前端据它算 dirty，
/// 否则草稿里缺的「后端专管」字段（同步来的费率、上下文上限）会让哈希算错，
/// 表现为点完「应用」仍显示「尚未应用」。
#[tauri::command]
pub fn save_config(state: State<'_, Arc<ProxyState>>, mut config: Config) -> Result<Config, String> {
    // applied 哈希/时间/端口由后端专管（apply_to_claude / set_port 里更新），
    // 忽略前端回传值防止漂移。
    {
        let cur = state.config.read().unwrap_or_else(|e| e.into_inner());
        config.last_applied_hash = cur.last_applied_hash.clone();
        config.last_applied_at = cur.last_applied_at.clone();
        config.last_applied_pool = cur.last_applied_pool.clone();
        config.last_applied_port = cur.last_applied_port;
        config.port = cur.port;
        config.pricing_synced_at = cur.pricing_synced_at.clone();
        config.models_dev_models = cur.models_dev_models.clone();
        config.models_dev_context = cur.models_dev_context.clone();
        // 后台同步可能刚写完，而前端手上这份草稿是同步前的 —— 别让它抹掉同步结果
        models_dev::preserve_synced_pricing(&mut config, &cur);
    }
    // 刚挑的模型：同步来的上下文上限先补上，1M 开关马上就能判断
    models_dev::fill_known_context(&mut config);
    save_config_file(&config)?;
    *state.config.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
    eprintln!("[config] saved");
    Ok(config)
}

/// 规范化配置摘要（design.md §8 应用状态机的 dirty 判定，前后端共用同一实现）。
#[tauri::command]
pub fn config_hash(config: Config) -> String {
    canonical_hash(&config)
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

/// 连接测试：1 token 试探请求（平移 v1 test_handler，话术不变）。
#[tauri::command]
pub async fn test_provider(
    state: State<'_, Arc<ProxyState>>,
    target_url: String,
    api_key: String,
    model: String,
) -> Result<TestResult, String> {
    if target_url.is_empty() || api_key.is_empty() || model.is_empty() {
        return Ok(TestResult {
            ok: false,
            message: "Please fill in URL, Key, and model name.".into(),
        });
    }
    if !target_url.starts_with("http://") && !target_url.starts_with("https://") {
        return Ok(TestResult {
            ok: false,
            message: "URL must start with http:// or https://".into(),
        });
    }

    let base = target_url.trim_end_matches('/');
    let url = format!("{}/v1/messages", base);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let test_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| state.client.clone());

    let resp = test_client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", api_key))
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_vec(&body).unwrap_or_default())
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            if status == 200 {
                Ok(TestResult {
                    ok: true,
                    message: format!("Connection successful! (HTTP {})", status),
                })
            } else {
                let body = r.text().await.unwrap_or_default();
                let msg = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .map(String::from)
                    })
                    .unwrap_or_else(|| format!("HTTP {}", status));
                Ok(TestResult { ok: false, message: msg })
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
            Ok(TestResult { ok: false, message: msg })
        }
    }
}

/// 应用到 Claude Desktop：校验 → 写网关 → 更新 applied 哈希 → 重启 Claude。
/// （前端在调用前先 flush 自动保存，保证 state 里是最新配置。）
#[tauri::command]
pub async fn apply_to_claude(state: State<'_, Arc<ProxyState>>) -> Result<String, String> {
    let mut config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    let msg = gateway::apply_to_claude_desktop(&config)?;
    eprintln!("[apply] {}", msg);

    // design.md §8：apply 成功后持久化 last_applied_hash（写盘失败不回滚 apply，仅打日志）
    config.last_applied_hash = canonical_hash(&config);
    config.last_applied_pool = crate::config::SLOT_POOL_VERSION.to_string();
    config.last_applied_port = Some(config.port);
    config.last_applied_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();
    if let Err(e) = save_config_file(&config) {
        eprintln!("[apply] WARN: persist last_applied_hash failed: {}", e);
    }
    *state.config.write().unwrap_or_else(|e| e.into_inner()) = config;

    gateway::restart_claude_desktop();
    Ok("Applied! Claude Desktop is restarting...".to_string())
}

#[derive(Serialize)]
pub struct DesktopInfo {
    /// 检测到的 Claude Desktop 版本；null = 没探到（没装 / 装在非常规位置）。
    pub version: Option<String>,
    /// 因版本过低而写不了的键（§3.8），设置页据此告诉用户哪些能力不可用。
    pub unavailable: Vec<String>,
}

/// 设置页展示用：检测到的 Claude Desktop 版本 + 受版本限制的能力（§3.8）。
#[tauri::command]
pub fn desktop_info() -> DesktopInfo {
    let gate = crate::desktop_version::VersionGate::detect();
    DesktopInfo {
        version: gate.version.clone(),
        unavailable: gate.unavailable_keys().iter().map(|s| s.to_string()).collect(),
    }
}

/// Claude Desktop 实际在用的网关配置（只读）：概览页逐槽位的「已生效 / 未应用」据此判断。
#[tauri::command]
pub fn applied_state() -> gateway::AppliedState {
    gateway::read_applied_state()
}

/// 「要不要应用」里逐槽位比对管不到的部分：网关地址、端口换没换过、费率表。
/// 只改密钥 / 地址 / 默认档这类代理当场就用上的东西，这里和槽位比对都不会报 ——
/// 不必为它们重启 Claude。
#[tauri::command]
pub fn pending_apply(state: State<'_, Arc<ProxyState>>) -> gateway::PendingApply {
    let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    gateway::read_pending_apply(&config)
}

/// 设置页「打开配置目录」：在访达 / 资源管理器里选中 Claude Desktop 正在用的那份配置文件。
/// 出问题时让用户把它发过来 —— 这个文件里没有 API 密钥（网关密钥固定写的是 "proxy"）。
#[tauri::command]
pub fn reveal_claude_config() -> Result<(), String> {
    let target = gateway::applied_config_file()
        .filter(|p| p.exists())
        .or_else(|| gateway::claude_3p_dir().filter(|d| d.exists()))
        .ok_or("没找到 Claude Desktop 的配置目录 —— Claude Desktop 可能还没装，或者还没打开过")?;
    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct PricingSyncResult {
    pub ok: bool,
    /// 费率发生变化的模型条数（0 = 已是最新）。
    pub changed: usize,
    /// 因「关了自动同步」或「距上次不足 6 小时」而没真拉。
    pub skipped: bool,
    pub message: String,
    pub synced_at: String,
}

/// 从 models.dev 同步费率（§3.1）。`force` = 用户手点，无视开关与 6 小时阈值。
///
/// 失败不影响任何既有配置：拉不到就保持上次同步的值（降级到「价格略旧」而不是「没有价格」）。
#[tauri::command]
pub async fn sync_pricing(
    state: State<'_, Arc<ProxyState>>,
    force: bool,
) -> Result<PricingSyncResult, String> {
    let (auto, synced_at) = {
        let c = state.config.read().unwrap_or_else(|e| e.into_inner());
        (c.pricing_auto_sync, c.pricing_synced_at.clone())
    };
    let now = models_dev::now_secs();
    if !force && (!auto || !models_dev::is_stale(&synced_at, now)) {
        return Ok(PricingSyncResult {
            ok: true,
            changed: 0,
            skipped: true,
            message: String::new(),
            synced_at,
        });
    }

    // 网络往返期间不持锁 —— 拿到 catalog 再回来落盘
    let (catalog, model_index, context_index) = match models_dev::fetch_catalog(&state.client).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[pricing] 同步失败: {}", e);
            return Ok(PricingSyncResult {
                ok: false,
                changed: 0,
                skipped: false,
                message: e,
                synced_at,
            });
        }
    };

    let (changed, config) = {
        let mut cur = state.config.write().unwrap_or_else(|e| e.into_inner());
        let mut changed = models_dev::apply_catalog(&mut cur, &catalog);
        cur.models_dev_models = model_index;
        cur.models_dev_context = context_index;
        changed += models_dev::fill_known_context(&mut cur);
        cur.pricing_synced_at = now.to_string();
        (changed, cur.clone())
    };
    if let Err(e) = save_config_file(&config) {
        eprintln!("[pricing] WARN: 同步结果落盘失败: {}", e);
    }
    eprintln!("[pricing] 同步完成：{} 家服务商，{} 个模型费率有变化", catalog.len(), changed);
    Ok(PricingSyncResult {
        ok: true,
        changed,
        skipped: false,
        message: String::new(),
        synced_at: now.to_string(),
    })
}

#[derive(Serialize)]
pub struct AvailableModel {
    pub id: String,
    /// 上下文上限（token）；models.dev 没写就是 None
    pub context: Option<u64>,
}

/// 这个服务商当前提供哪些模型（models.dev 数据，按发布日期新→旧），
/// 供模型选择器列出。认不出这家服务商、或还没同步过时返回空。
#[tauri::command]
pub fn available_models(state: State<'_, Arc<ProxyState>>, target_url: String) -> Vec<AvailableModel> {
    let c = state.config.read().unwrap_or_else(|e| e.into_inner());
    let Some(pid) = models_dev::provider_id_for_url(&target_url) else {
        return Vec::new();
    };
    let contexts = c.models_dev_context.get(pid);
    c.models_dev_models
        .get(pid)
        .map(|ids| {
            ids.iter()
                .map(|id| AvailableModel { id: id.clone(), context: contexts.and_then(|m| m.get(id)).copied() })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_logs(state: State<'_, Arc<ProxyState>>) -> Vec<LogEntry> {
    state.logs.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 日志页顶部摘要：今天的成功率 / 耗时 / token / 花费（不受「只留 100 条」限制）。
#[tauri::command]
pub fn get_log_stats(state: State<'_, Arc<ProxyState>>) -> TodayStats {
    state.today_stats()
}

#[derive(Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
}

fn read_status(state: &ProxyState) -> ProxyStatus {
    use std::sync::atomic::Ordering;
    let running = state.running.load(Ordering::SeqCst);
    let port = if running {
        state.bound_port.load(Ordering::SeqCst)
    } else {
        state.config.read().unwrap_or_else(|e| e.into_inner()).port
    };
    ProxyStatus { running, port }
}

/// 侧栏状态块数据：代理是否在监听 + 端口。
#[tauri::command]
pub fn proxy_status(state: State<'_, Arc<ProxyState>>) -> ProxyStatus {
    read_status(&state)
}

/// 端口热切换（2026-07-14 用户拍板）：先绑新端口再放旧的，失败不影响现有服务；
/// 成功后持久化 config.port 并立即改写 Claude 网关 URL（模型列表沿用，须重新应用重启 Claude）。
#[tauri::command]
pub async fn set_port(state: State<'_, Arc<ProxyState>>, port: u16) -> Result<ProxyStatus, String> {
    use std::sync::atomic::Ordering;

    if port < 1024 {
        return Err("端口需在 1024–65535 之间".into());
    }
    if state.running.load(Ordering::SeqCst) && state.bound_port.load(Ordering::SeqCst) == port {
        return Ok(read_status(&state));
    }

    let listener = crate::proxy::bind(port).await?;
    state.inner().start_serving(listener, port);

    let mut config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
    config.port = port;
    save_config_file(&config)?;
    *state.config.write().unwrap_or_else(|e| e.into_inner()) = config;

    gateway::ensure_claude_desktop_gateway(port);
    eprintln!("[port] switched to 127.0.0.1:{}", port);
    Ok(read_status(&state))
}

/// 自动更新装完后的可靠重启（移植 ClaudeCN）：兜底 Tauri v2 在 macOS 上 relaunch()
/// 的已知 bug（装好新包却没能重启）。spawn 一个脱离的 helper 轮询父进程退出后再 `open -n` 重开。
#[tauri::command]
pub fn force_quit_and_relaunch(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let current_exe =
            std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let ppid = std::process::id();
        let app_bundle = current_exe
            .ancestors()
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("app"))
            .ok_or_else(|| "current_exe 祖先里没有 .app bundle".to_string())?;
        let escaped = format!("'{}'", app_bundle.to_string_lossy().replace('\'', "'\\''"));
        let cmd = format!(
            "i=0; while kill -0 {ppid} 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done; sleep 0.3; open -n {escaped}"
        );
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn relaunch helper failed: {e}"))?;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            app.exit(0);
        });
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // windows/linux 上 plugin-process 的 relaunch 没有那个 bug；这里直接 restart（不再返回）
        app.restart()
    }
}
