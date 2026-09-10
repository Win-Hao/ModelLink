//! 本地代理（127.0.0.1:5678）：/v1/models 列表 + 消息流式转发。
//! 平移 v1 claude-model-proxy main.rs:642-1065 —— 行为逐字节等价，禁止顺手优化。
//! 与 v1 的差异仅有：不再 serve UI 与 /api/*（那些职责移交 Tauri 命令层）。
//!
//! ⚠️ 兼容红线（docs/gui-rebuild-tauri.md §3 #1/#4）：
//! - 端口 5678 不变；被占时报 v1 原话术并退出
//! - anthropic-beta / x-api-key / user-agent 透传、/v1/models 响应格式、
//!   流式转发、10MB body 上限
//!
//! 2.1-A 有意偏离 v1 的两处（docs/design-2.1-follow-desktop.md）：
//! - §3.10 thinking 注入改三态：桌面端自带 output_config.effort 时原样透传，
//!   服务商级 thinking_effort 降级为「桌面端未指定时的默认档位」
//! - §3.11.1 effort 整流：上游拒绝 output_config 时代理层移除后重试一次

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Json},
    Router,
};
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::net::TcpListener;

use crate::config::{flatten_config, resolve_model, Config, ResolvedModel};

pub const MAX_LOGS: usize = 100;

/// 单请求最多整流次数（§3.11 通用约束，防循环）。同类整流每请求只做一次。
pub(crate) const MAX_RECTIFY: usize = 2;

#[derive(Serialize, Clone)]
pub struct LogEntry {
    pub time: String,
    pub model: String,
    pub status: u16,
    /// 本次实际发给上游的推理强度（""=未发 / "off" / low…max）。
    pub thinking: String,
    /// 2.1-A 附注：整流标记、未映射槽位等。多条以 " · " 连接，空 = 无附注。
    pub note: String,
    /// 2.1-A：ModelLink 自己判定为错误的请求（日志页标红）。
    pub error: bool,
}

/// 服务商思考能力缓存（§3.11.4）。A 批只用到 `effort_supported`，
/// 其余字段随 §3.11.2/3 的整流器一起补。
///
/// ⚠️ §3.11.5：按「服务商 URL + 真实上游模型 ID」索引 —— 槽位名是借用来的
/// Claude 型号名，与背后真实模型毫无关系，绝不能从槽位名推断上游能力。
/// A 批只存内存（进程内有效）；随 ProviderPreset 持久化是后续批次的事。
#[derive(Default, Clone, Copy)]
pub struct ThinkingCaps {
    /// 上游是否接受 `output_config.effort`。None = 未知（乐观发送）。
    pub effort_supported: Option<bool>,
}

/// 代理与命令层共享的全局状态（与 v1 AppState 同构 + 2.0 运行态：端口热切换）。
pub struct ProxyState {
    pub config: RwLock<Config>,
    pub client: Client,
    pub logs: RwLock<Vec<LogEntry>>,
    /// 代理是否在监听（端口被占时为 false，app 不退出，侧栏显示未运行）。
    pub running: AtomicBool,
    /// 实际绑定的端口（未运行时无意义）。
    pub bound_port: AtomicU16,
    /// 当前 serve 任务句柄，切换端口时 abort 旧任务释放监听。
    pub serve_handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// 上游思考能力缓存（§3.11.4），键 = (服务商 URL, 真实上游模型 ID)。
    pub caps: RwLock<HashMap<(String, String), ThinkingCaps>>,
}

impl ProxyState {
    pub fn new(config: Config) -> Result<Self, String> {
        Ok(Self {
            config: RwLock::new(config),
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?,
            logs: RwLock::new(Vec::new()),
            running: AtomicBool::new(false),
            bound_port: AtomicU16::new(0),
            serve_handle: Mutex::new(None),
            caps: RwLock::new(HashMap::new()),
        })
    }

    /// 在已绑定的 listener 上启动 serve 任务并登记运行态（替换旧任务）。
    pub fn start_serving(self: &Arc<Self>, listener: TcpListener, port: u16) {
        let mut guard = self.serve_handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = guard.take() {
            old.abort();
        }
        let state = self.clone();
        *guard = Some(tauri::async_runtime::spawn(serve(listener, state)));
        self.running.store(true, Ordering::SeqCst);
        self.bound_port.store(port, Ordering::SeqCst);
    }

    /// 该服务商 + 上游模型是否已知不认 `output_config.effort`（§3.11.1 副作用）。
    pub fn effort_unsupported(&self, url: &str, model: &str) -> bool {
        self.caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(url.to_string(), model.to_string()))
            .and_then(|c| c.effort_supported)
            == Some(false)
    }

    /// 整流成功后落一笔：后续请求直接不发 effort，省掉每次的失败往返。
    pub fn mark_effort_unsupported(&self, url: &str, model: &str) {
        self.caps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry((url.to_string(), model.to_string()))
            .or_default()
            .effort_supported = Some(false);
    }
}

pub fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let offset_secs: i64 = {
        #[cfg(target_os = "macos")]
        {
            let mut now: libc::time_t = 0;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            unsafe {
                libc::time(&mut now);
                libc::localtime_r(&now, &mut tm);
            }
            tm.tm_gmtoff
        }
        #[cfg(not(target_os = "macos"))]
        {
            8 * 3600
        }
    };
    let local = (d as i64 + offset_secs) as u64;
    let h = (local % 86400) / 3600;
    let m = (local % 3600) / 60;
    let s = local % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// /v1/models 响应体（从 v1 proxy_fallback 内联代码原样抽出，便于单测与回归 diff）。
pub(crate) fn models_json(config: &Config) -> serde_json::Value {
    let flat = flatten_config(config);
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
    serde_json::json!({ "data": models })
}

/// 请求体里桌面端自带的推理强度（`output_config.effort`）。
/// Claude Desktop 1.46388.3 起对 `Vwt` 表内的槽位原生渲染 5 档选择器，
/// 自己把选中的档位写进请求体（同时带 `thinking:{"type":"adaptive"}`）。
pub(crate) fn request_effort(data: &serde_json::Value) -> Option<String> {
    data.get("output_config")
        .and_then(|oc| oc.get("effort"))
        .and_then(|e| e.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// thinking 注入三态（§3.10 —— 语义相对 v1 反转：**透传优先，服务商级设置只兜底**）：
///
/// | 请求带 `output_config.effort` | `thinking_effort` | 行为 |
/// |---|---|---|
/// | ✅ 有 | 任意 | 原样透传，服务商级设置完全不参与 |
/// | ❌ 无 | `""` | 不注入，交给上游默认 |
/// | ❌ 无 | `"off"` | `thinking:{type:"disabled"}` + 移除 `output_config` |
/// | ❌ 无 | 其它档位 | 注入该档位作为兜底默认 |
///
/// 返回写入请求日志的 thinking 标签（= 实际发给上游的档位）。
pub(crate) fn inject_thinking(data: &mut serde_json::Value, te: &str) -> String {
    // ① 桌面端已经指定档位 —— 用户在选择器里的选择优先级最高，一个字节都不动。
    //    （v1 在这里无条件覆盖，把桌面端的 effort 和 adaptive 思考一起踩掉。）
    if let Some(effort) = request_effort(data) {
        eprintln!("  effort: {} (桌面端指定，透传)", effort);
        return effort;
    }

    // ② 以下均为「桌面端未指定」的兜底路径。
    if te.is_empty() {
        return String::new();
    }

    if te == "off" {
        // 关闭思考是用户在服务商上的显式指令，且此时请求里没有 effort 可尊重，
        // 因此这条允许改写已有 thinking（否则「关闭思考」永远无法生效）。
        data["thinking"] = serde_json::json!({"type": "disabled"});
        data.as_object_mut().map(|o| o.remove("output_config"));
        eprintln!("  thinking: disabled (服务商默认)");
        return "off".to_string();
    }

    // 只写 effort 键，不整体替换 output_config（避免踩掉其它未知字段）。
    match data.get_mut("output_config") {
        Some(serde_json::Value::Object(oc)) => {
            oc.insert("effort".to_string(), serde_json::json!(te));
        }
        _ => data["output_config"] = serde_json::json!({"effort": te}),
    }
    // 绝不覆盖请求里已有的 thinking（尤其 {"type":"adaptive"}）。
    // budget_tokens 与 effort 语义重叠、对 1M 上下文模型也偏小，
    // 只在这条兜底路径、且请求完全没带 thinking 时才写。
    if data.get("thinking").is_none() {
        data["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": 8192});
    }
    eprintln!("  thinking_effort: {} (服务商默认)", te);
    te.to_string()
}

/// 整流器种类（§3.11）。A 批只实现 effort（§3.11.1）；
/// §3.11.2 budget 约束 / §3.11.3 thinking 块签名后续批次接进同一张表。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Rectifier {
    /// 上游不接受 `output_config.effort` → 移除后重试（§3.11.1）。
    Effort,
}

impl Rectifier {
    /// 整流成功后写进请求日志的标记。
    pub(crate) fn success_note(self) -> &'static str {
        match self {
            Rectifier::Effort => "已自动修复：effort 不支持",
        }
    }

    /// 整流失败时附在原始错误上的「尝试过的修复」。
    pub(crate) fn attempt_note(self) -> &'static str {
        match self {
            Rectifier::Effort => "移除 output_config.effort",
        }
    }
}

/// 从上游错误体里取 `error.message`；不是标准 Anthropic 错误体时退回原文。
pub(crate) fn upstream_error_message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string())
}

/// §3.11.1 触发判定。两条路：
/// - 错误消息明确提到 `output_config` / `effort` —— 确定命中；
/// - 形态未知的兼容端点只给一句泛泛的 400 —— 投机重试一次，
///   「去掉 output_config 后即成功」才反过来认定它不支持（见调用处：只有重试
///   成功才写能力缓存）。401/403/404/429 显然与请求体无关，不投机。
pub(crate) fn effort_rectifier_applies(status: u16, msg: &str, has_output_config: bool) -> bool {
    if !has_output_config || !(400..500).contains(&status) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("output_config") || m.contains("effort") || status == 400
}

/// 移除 `output_config`；返回是否真的移除了东西（没得改就别算一次整流）。
pub(crate) fn strip_output_config(data: &mut serde_json::Value) -> bool {
    data.as_object_mut()
        .and_then(|o| o.remove("output_config"))
        .is_some()
}

/// 整流失败时把「尝试过的修复」附到原始错误消息后面。
/// 只在上游给的是标准 Anthropic 错误体时改写，否则返回 None（原样透传，不破坏响应）。
pub(crate) fn annotate_error_body(body: &[u8], attempted: &[&str]) -> Option<Vec<u8>> {
    if attempted.is_empty() {
        return None;
    }
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let msg = v.get("error")?.get("message")?.as_str()?.to_string();
    v["error"]["message"] = serde_json::json!(format!(
        "{}\n\n[ModelLink 尝试过的自动修复：{}（仍失败）]",
        msg,
        attempted.join("、")
    ));
    serde_json::to_vec(&v).ok()
}

/// 上游响应头透传过滤（逐跳头不转发）。
fn passthrough_headers(src: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (k, v) in src {
        if k != "transfer-encoding" && k != "connection" {
            headers.insert(k.clone(), v.clone());
        }
    }
    headers
}

fn push_log(state: &ProxyState, entry: LogEntry) {
    let mut logs = state.logs.write().unwrap_or_else(|e| e.into_inner());
    logs.push(entry);
    let len = logs.len();
    if len > MAX_LOGS {
        logs.drain(0..len - MAX_LOGS);
    }
}

async fn proxy_fallback(
    State(state): State<Arc<ProxyState>>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();

    if parts.method == Method::GET && parts.uri.path().contains("/v1/models") {
        let config = state.config.read().unwrap_or_else(|e| e.into_inner()).clone();
        return Json(models_json(&config)).into_response();
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

    // thinking 三态注入（§3.10：透传优先，服务商级设置只兜底）
    let mut thinking_log = inject_thinking(&mut data, &resolved.thinking_effort);
    let mut notes: Vec<&'static str> = Vec::new();

    if resolved.target_url.is_empty() {
        eprintln!("  error: no target URL configured for this model");
        return (StatusCode::BAD_GATEWAY, "No API URL configured for this model. Please configure the provider in the proxy app.").into_response();
    }

    // 能力缓存命中：这家上游已知不认 output_config.effort，直接不发，
    // 省掉每次的失败往返（§3.11.1 副作用）。
    if state.effort_unsupported(&resolved.target_url, &resolved.model)
        && strip_output_config(&mut data)
    {
        eprintln!("  effort: 上游已知不支持，本次不发送");
        notes.push("effort 不支持（已缓存）");
        thinking_log = String::new();
    }

    let base = resolved.target_url.trim_end_matches('/');
    let url = format!("{}{}", base, parts.uri.path());

    // 整流后要用同一组头重发一次，请求构造抽成闭包。
    let build = |body: Vec<u8>| {
        let mut b = state
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
                b = b.header(h, v);
            }
        }
        b.body(body)
    };

    // 已应用的整流器（同类只做一次、单请求最多 MAX_RECTIFY 次 —— §3.11 防循环）
    let mut applied: Vec<Rectifier> = Vec::new();
    // 首次错误：整流失败时原样返回它，而不是重试后的新错误
    let mut first_error: Option<(u16, HeaderMap, Vec<u8>)> = None;

    let (raw_status, headers, err_body) = loop {
        let resp = match build(serde_json::to_vec(&data).unwrap_or_default()).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  proxy error: {}", e);
                return (StatusCode::BAD_GATEWAY, format!("Proxy error: {}", e)).into_response();
            }
        };

        let raw_status = resp.status().as_u16();

        // 非 4xx：与 v1 一致的流式直通，一个字节都不缓冲。
        if !(400..500).contains(&raw_status) {
            for r in &applied {
                if raw_status < 400 {
                    // 整流后这一发成了：记住能力，后续请求直接不发（§3.11.1 副作用）
                    match r {
                        Rectifier::Effort => {
                            state.mark_effort_unsupported(&resolved.target_url, &resolved.model)
                        }
                    }
                    notes.push(r.success_note());
                } else {
                    // 5xx：换了个失败方式，不足以断定上游不支持 —— 不写缓存，但要留痕
                    notes.push(match r {
                        Rectifier::Effort => "已尝试修复：effort",
                    });
                }
            }
            let status =
                StatusCode::from_u16(raw_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let headers = passthrough_headers(resp.headers());
            if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
                push_log(
                    state.as_ref(),
                    LogEntry {
                        time: chrono_now(),
                        model: model.to_string(),
                        status: raw_status,
                        thinking: thinking_log.clone(),
                        note: notes.join(" · "),
                        error: false,
                    },
                );
            }
            return (status, headers, Body::from_stream(resp.bytes_stream())).into_response();
        }

        // 4xx：此时下游一个字节都还没写出去，所以「已吐出 SSE 事件的请求不重试」
        // 这条约束天然满足。缓冲错误体做整流判定。
        let headers = passthrough_headers(resp.headers());
        let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        let msg = upstream_error_message(&body);

        // §3.11.1 effort 整流
        if applied.len() < MAX_RECTIFY
            && !applied.contains(&Rectifier::Effort)
            && effort_rectifier_applies(raw_status, &msg, data.get("output_config").is_some())
            && strip_output_config(&mut data)
        {
            eprintln!("  rectify: 移除 output_config 后重试（上游 {}: {}）", raw_status, msg);
            applied.push(Rectifier::Effort);
            thinking_log = String::new();
            first_error.get_or_insert((raw_status, headers, body));
            continue;
        }

        // 无整流可做（或整流后仍失败）→ 原样返回最初的错误。
        break first_error.take().unwrap_or((raw_status, headers, body));
    };

    // 走到这里说明最终仍是 4xx：整流（若有）没能救回来。
    if let Some(r) = applied.last() {
        eprintln!("  rectify: 未生效，原样返回上游错误");
        notes.push(match r {
            Rectifier::Effort => "自动修复未生效：effort",
        });
    }

    let status = StatusCode::from_u16(raw_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
        push_log(
            state.as_ref(),
            LogEntry {
                time: chrono_now(),
                model: model.to_string(),
                status: raw_status,
                thinking: thinking_log.clone(),
                note: notes.join(" · "),
                error: false,
            },
        );
    }

    let attempted: Vec<&str> = applied.iter().map(|r| r.attempt_note()).collect();
    match annotate_error_body(&err_body, &attempted) {
        Some(annotated) => {
            // 改写过消息，上游的 content-length 不再成立，交给 axum 重算。
            let mut headers = headers;
            headers.remove("content-length");
            (status, headers, Body::from(annotated)).into_response()
        }
        None => (status, headers, Body::from(err_body)).into_response(),
    }
}

/// 绑定端口。失败时返回 v1 原话术（含「请先关闭另一个实例」语义），由调用方提示。
pub async fn bind(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| format!("Port {} already in use: {}. Please close the other instance first.", port, e))
}

/// 在已绑定的 listener 上常驻服务。axum 只保留代理职责：/v1/models + fallback 转发。
pub async fn serve(listener: TcpListener, state: Arc<ProxyState>) {
    let app = Router::new().fallback(proxy_fallback).with_state(state);
    eprintln!("Server ready.");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Server error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelEntry, Provider};

    fn cfg_one(te: &str) -> Config {
        Config {
            providers: vec![Provider {
                target_url: "https://a.example.com".into(),
                api_key: "k".into(),
                models: vec![
                    ModelEntry { name: "m-with-1m".into(), to_1m: "auto".into() },
                    ModelEntry { name: "m-plain".into(), to_1m: "".into() },
                ],
                thinking_effort: te.into(),
            }],
            ..Default::default()
        }
    }

    // ---- /v1/models 响应格式（红线 #4：与旧版 diff 一致） ----

    #[test]
    fn models_json_lists_slots_and_1m_variants() {
        let v = models_json(&cfg_one(""));
        let data = v.get("data").unwrap().as_array().unwrap();
        assert_eq!(data.len(), 3); // slot1 + slot1[1m] + slot2
        assert_eq!(data[0]["id"], "claude-3-opus-latest");
        assert_eq!(data[0]["display_name"], "m-with-1m");
        assert_eq!(data[0]["created"], 0);
        assert_eq!(data[1]["id"], "claude-3-opus-latest[1m]");
        assert_eq!(data[1]["display_name"], "m-with-1m (1M)");
        assert_eq!(data[2]["id"], "claude-3-5-sonnet-latest");
        assert_eq!(data[2]["display_name"], "m-plain");
    }

    #[test]
    fn models_json_empty_config_gives_empty_data() {
        let v = models_json(&Config::default());
        assert_eq!(v, serde_json::json!({ "data": [] }));
    }

    // ---- §3.10 thinking 注入三态（四个分支各一例） ----

    // 分支 ①：请求里已有 output_config.effort → 原样透传，服务商级设置完全不参与。
    #[test]
    fn inject_thinking_passes_through_desktop_effort_untouched() {
        // 桌面端 1.46388.3 的真实形态：effort + adaptive 思考（§5.5.2 抓包）
        let orig = serde_json::json!({
            "model": "m",
            "output_config": {"effort": "xhigh"},
            "thinking": {"type": "adaptive"}
        });
        // 服务商级设的每一档都不许改写它
        for te in ["", "off", "low", "high", "max"] {
            let mut data = orig.clone();
            let tag = inject_thinking(&mut data, te);
            assert_eq!(tag, "xhigh", "te={te} 的日志标签应是桌面端档位");
            assert_eq!(data, orig, "te={te} 时请求体被改写了");
        }
    }

    // 分支 ②：无 effort + thinking_effort=""  → 不注入，交给上游默认。
    #[test]
    fn inject_thinking_default_leaves_body_untouched() {
        let mut data = serde_json::json!({"model": "m", "max_tokens": 1});
        let tag = inject_thinking(&mut data, "");
        assert_eq!(tag, "");
        assert_eq!(data, serde_json::json!({"model": "m", "max_tokens": 1}));
    }

    // 分支 ③：无 effort + thinking_effort="off" → disabled + 移除 output_config。
    #[test]
    fn inject_thinking_off_disables_and_strips_output_config() {
        // output_config 存在但没有 effort（不构成「桌面端已指定」）
        let mut data = serde_json::json!({"model": "m", "output_config": {"other": 1}});
        let tag = inject_thinking(&mut data, "off");
        assert_eq!(tag, "off");
        assert_eq!(data["thinking"], serde_json::json!({"type": "disabled"}));
        assert!(data.get("output_config").is_none());
    }

    // 分支 ④：无 effort + 其它档位 → 注入该档位作为兜底默认。
    #[test]
    fn inject_thinking_falls_back_to_provider_level_effort() {
        let mut data = serde_json::json!({"model": "m"});
        let tag = inject_thinking(&mut data, "high");
        assert_eq!(tag, "high");
        assert_eq!(data["output_config"], serde_json::json!({"effort": "high"}));
        // 请求完全没带 thinking 时才补 budget_tokens 兜底
        assert_eq!(
            data["thinking"],
            serde_json::json!({"type": "enabled", "budget_tokens": 8192})
        );

        let mut data = serde_json::json!({"model": "m"});
        assert_eq!(inject_thinking(&mut data, "max"), "max");
        assert_eq!(data["output_config"], serde_json::json!({"effort": "max"}));
    }

    // 兜底注入也绝不覆盖请求里已有的 thinking（尤其 adaptive）。
    #[test]
    fn inject_thinking_never_overwrites_existing_thinking_block() {
        let mut data = serde_json::json!({"model": "m", "thinking": {"type": "adaptive"}});
        let tag = inject_thinking(&mut data, "high");
        assert_eq!(tag, "high");
        assert_eq!(data["thinking"], serde_json::json!({"type": "adaptive"}));
        assert_eq!(data["output_config"], serde_json::json!({"effort": "high"}));
    }

    // 兜底注入只写 effort 键，不整体替换 output_config。
    #[test]
    fn inject_thinking_merges_into_existing_output_config() {
        let mut data = serde_json::json!({"model": "m", "output_config": {"keep": true}});
        inject_thinking(&mut data, "low");
        assert_eq!(
            data["output_config"],
            serde_json::json!({"keep": true, "effort": "low"})
        );
    }

    #[test]
    fn request_effort_ignores_empty_and_missing() {
        assert_eq!(request_effort(&serde_json::json!({})), None);
        assert_eq!(request_effort(&serde_json::json!({"output_config": {}})), None);
        assert_eq!(
            request_effort(&serde_json::json!({"output_config": {"effort": ""}})),
            None
        );
        assert_eq!(
            request_effort(&serde_json::json!({"output_config": {"effort": "medium"}})),
            Some("medium".to_string())
        );
    }

    // ---- §3.11.1 effort 整流 ----

    #[test]
    fn effort_rectifier_matches_explicit_error_messages() {
        assert!(effort_rectifier_applies(
            400,
            "output_config: extra inputs are not permitted",
            true
        ));
        assert!(effort_rectifier_applies(422, "unknown field `effort`", true));
        // 大小写不敏感
        assert!(effort_rectifier_applies(422, "Unsupported EFFORT level", true));
    }

    #[test]
    fn effort_rectifier_speculates_only_on_plain_400() {
        // 形态未知的 400 → 投机重试一次
        assert!(effort_rectifier_applies(400, "bad request", true));
        // 与请求体无关的 4xx → 不动
        assert!(!effort_rectifier_applies(401, "invalid api key", true));
        assert!(!effort_rectifier_applies(429, "rate limited", true));
        assert!(!effort_rectifier_applies(404, "not found", true));
    }

    #[test]
    fn effort_rectifier_skips_when_nothing_to_strip_or_not_4xx() {
        assert!(!effort_rectifier_applies(400, "effort bad", false));
        assert!(!effort_rectifier_applies(500, "effort bad", true));
        assert!(!effort_rectifier_applies(200, "effort bad", true));
    }

    #[test]
    fn strip_output_config_reports_whether_it_changed_anything() {
        let mut data = serde_json::json!({"model": "m", "output_config": {"effort": "max"}});
        assert!(strip_output_config(&mut data));
        assert_eq!(data, serde_json::json!({"model": "m"}));
        assert!(!strip_output_config(&mut data));
    }

    #[test]
    fn upstream_error_message_prefers_anthropic_shape() {
        let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"nope"}}"#;
        assert_eq!(upstream_error_message(body), "nope");
        // 非标准体退回原文
        assert_eq!(upstream_error_message(b"  boom  "), "boom");
    }

    #[test]
    fn annotate_error_body_appends_attempted_fixes() {
        let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"nope"}}"#;
        let out = annotate_error_body(body, &[Rectifier::Effort.attempt_note()]).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.starts_with("nope"));
        assert!(msg.contains("移除 output_config.effort"));
        // 其它字段不动
        assert_eq!(v["error"]["type"], "invalid_request_error");
    }

    #[test]
    fn annotate_error_body_leaves_non_anthropic_bodies_alone() {
        assert!(annotate_error_body(b"<html>502</html>", &["x"]).is_none());
        // 没整流过就不加注
        let body = br#"{"error":{"message":"nope"}}"#;
        assert!(annotate_error_body(body, &[]).is_none());
    }

    // ---- §3.11.4 能力缓存（按服务商 + 真实上游模型 ID，绝不按槽位名） ----

    #[test]
    fn caps_cache_is_keyed_by_provider_and_real_model() {
        let state = ProxyState::new(Config::default()).unwrap();
        assert!(!state.effort_unsupported("https://a.example.com", "real-a"));
        state.mark_effort_unsupported("https://a.example.com", "real-a");
        assert!(state.effort_unsupported("https://a.example.com", "real-a"));
        // 同服务商的另一个模型、另一家服务商的同名模型都不受影响
        assert!(!state.effort_unsupported("https://a.example.com", "real-b"));
        assert!(!state.effort_unsupported("https://b.example.com", "real-a"));
    }
}
