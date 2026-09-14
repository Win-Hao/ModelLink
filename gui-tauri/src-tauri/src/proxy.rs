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
//! - §3.3 未映射的槽位不再静默回落到第一个模型，改为 400 + Anthropic 错误体

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

use crate::config::{
    flatten_config, resolve_model, Config, ResolveError, ResolvedModel, HEARTBEAT_SECS,
};

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
    /// 上游是否吃得下历史消息里的 thinking / redacted_thinking 块（§3.11.3）。
    pub accepts_thinking_blocks: Option<bool>,
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

    /// 能力缓存的键。`[1m]` 变体与裸模型是同一个上游模型，能力必然相同 ——
    /// 不归一化就会造出两条互不相通的记录，「学一次别再浪费往返」的意义就没了。
    fn caps_key(url: &str, model: &str) -> (String, String) {
        (url.to_string(), model.strip_suffix("[1m]").unwrap_or(model).to_string())
    }

    /// 该服务商 + 上游模型是否已知不认 `output_config.effort`（§3.11.1 副作用）。
    pub fn effort_unsupported(&self, url: &str, model: &str) -> bool {
        self.caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&Self::caps_key(url, model))
            .and_then(|c| c.effort_supported)
            == Some(false)
    }

    /// 整流成功后落一笔：后续请求直接不发 effort，省掉每次的失败往返。
    pub fn mark_effort_unsupported(&self, url: &str, model: &str) {
        self.caps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(Self::caps_key(url, model))
            .or_default()
            .effort_supported = Some(false);
    }

    /// 该服务商 + 上游模型是否已知收不了历史 thinking 块（§3.11.3 副作用）。
    pub fn thinking_blocks_rejected(&self, url: &str, model: &str) -> bool {
        self.caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&Self::caps_key(url, model))
            .and_then(|c| c.accepts_thinking_blocks)
            == Some(false)
    }

    pub fn mark_thinking_blocks_rejected(&self, url: &str, model: &str) {
        self.caps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(Self::caps_key(url, model))
            .or_default()
            .accepts_thinking_blocks = Some(false);
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
    // 请求完全没带 thinking 时补一个 —— 补的是**桌面端同款形态**（§5.5.2 抓包：
    // output_config.effort 与 thinking:{"type":"adaptive"} 成对出现）。
    //
    // 这里曾经写死 {"type":"enabled","budget_tokens":8192}：那个数字在 v1 代码里是
    // 个没有出处的裸字面量，与 effort 语义重叠，对 1M 上下文模型也明显偏小。
    // adaptive 把「想多久」交回给上游，既不用编数字，也和桌面端保持一致。
    if data.get("thinking").is_none() {
        data["thinking"] = serde_json::json!({"type": "adaptive"});
    }
    eprintln!("  thinking_effort: {} (服务商默认)", te);
    te.to_string()
}

/// 会话标题生成请求的特征串（§5.5.4 抓包所见的原文开头）。
const TITLE_PROMPT_PREFIX: &str = "You are coming up with a succinct title";

/// 取首条消息的正文（字符串或 block 数组两种形态都认）。
fn first_message_text(data: &serde_json::Value) -> Option<String> {
    let first = data.get("messages")?.as_array()?.first()?;
    match first.get("content")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .find_map(|b| b.get("text").and_then(|t| t.as_str()))
            .map(String::from),
        _ => None,
    }
}

/// 这是不是「会话标题生成」（§5.5.4）。
///
/// 桌面端每开一个新会话都会来一发：451 in / 110 out，**其中 88 是思考 token** ——
/// 它不带 output_config，上游按自己的默认档跑（Kimi 默认 high），只为起个标题。
///
/// 只认首条 user 消息的开头，不看 max_tokens —— 后者是实现细节，容易随版本变。
pub(crate) fn is_title_generation(data: &serde_json::Value) -> bool {
    first_message_text(data)
        .map(|t| t.trim_start().starts_with(TITLE_PROMPT_PREFIX))
        .unwrap_or(false)
}

/// 给标题生成用最省的思考设置。返回是否真的改了。
///
/// 桌面端已经明确指定 effort 时不动 —— 与 §3.10「透传优先」同一条原则。
pub(crate) fn optimize_title_generation(data: &mut serde_json::Value) -> bool {
    if request_effort(data).is_some() {
        return false;
    }
    data["output_config"] = serde_json::json!({"effort": "low"});
    data["thinking"] = serde_json::json!({"type": "disabled"});
    true
}

/// 请求发出前的思考参数处理：标题生成优化（§5.5.4）+ thinking 三态注入（§3.10）。
/// 返回（写进请求日志的档位标签，是否做了标题优化）。
///
/// ⚠️ **顺序不能反**：标题优化里「桌面端已经选了就不动」那条豁免，看的必须是
/// **桌面端发来的** `output_config.effort`。若先跑 `inject_thinking`，服务商的默认档
/// 会被注进去，豁免条件随即成立 —— 于是只要用户在服务商上设过默认档，
/// 标题优化就永远不生效。先优化、再让 `inject_thinking` 的透传分支自然接手。
pub(crate) fn prepare_thinking(
    data: &mut serde_json::Value,
    provider_effort: &str,
) -> (String, bool) {
    let optimized = is_title_generation(data) && optimize_title_generation(data);
    let tag = inject_thinking(data, provider_effort);
    (tag, optimized)
}

/// 整流器种类（§3.11）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Rectifier {
    /// 上游不接受 `output_config.effort` → 移除后重试（§3.11.1）。
    Effort,
    /// 上游对 thinking budget 有下限要求 → 抬到 32000 后重试（§3.11.2）。
    ThinkingBudget,
    /// 上游验不了历史消息里的 thinking 块签名 → 剥掉后重试（§3.11.3）。
    ThinkingBlocks,
}

impl Rectifier {
    /// 整流成功后写进请求日志的标记。
    pub(crate) fn success_note(self) -> &'static str {
        match self {
            Rectifier::Effort => "已自动修复：effort 不支持",
            Rectifier::ThinkingBudget => "已自动修复：thinking budget 下限",
            Rectifier::ThinkingBlocks => "已自动修复：thinking 块签名",
        }
    }

    /// 整流失败时附在原始错误上的「尝试过的修复」。
    pub(crate) fn attempt_note(self) -> &'static str {
        match self {
            Rectifier::Effort => "移除 output_config.effort",
            Rectifier::ThinkingBudget => "抬高 thinking.budget_tokens",
            Rectifier::ThinkingBlocks => "移除历史 thinking 块与签名",
        }
    }

    /// 整流没救回来时写进日志的标记。
    pub(crate) fn failed_note(self) -> &'static str {
        match self {
            Rectifier::Effort => "自动修复未生效：effort",
            Rectifier::ThinkingBudget => "自动修复未生效：budget",
            Rectifier::ThinkingBlocks => "自动修复未生效：thinking 块",
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

/// §3.11.1 触发判定之一：错误消息明确点名 `output_config` / `effort` —— 确定命中。
pub(crate) fn effort_error_explicit(status: u16, msg: &str, has_output_config: bool) -> bool {
    if !has_output_config || !(400..500).contains(&status) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("output_config") || m.contains("effort")
}

/// §3.11.1 触发判定之二：形态未知的兼容端点只给一句泛泛的 400 —— 投机重试一次。
/// 「去掉 output_config 后即成功」才反过来认定它不支持（调用处只在重试成功时写
/// 能力缓存）。401/403/404/429 显然与请求体无关，不投机。
///
/// ⚠️ 必须**排在其它整流器之后**判定：budget / thinking 块的报错同样是 400，
/// 让投机分支先命中就会去删一个跟错误无关的字段，白白浪费一次整流额度。
pub(crate) fn effort_error_speculative(status: u16, has_output_config: bool) -> bool {
    has_output_config && status == 400
}

/// §3.11.2 触发判定：错误消息**同时**含 budget 字样 + thinking + 1024 下限约束。
///
/// 例外：`thinking.type == "adaptive"` 的请求不改写 —— 自适应思考没有 budget 概念，
/// 硬塞一个 budget_tokens 只会换来一个新错误。
pub(crate) fn budget_rectifier_applies(status: u16, msg: &str, data: &serde_json::Value) -> bool {
    if !(400..500).contains(&status) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    let has_thinking = m.contains("thinking");
    let is_adaptive =
        data.get("thinking").and_then(|t| t.get("type")).and_then(|t| t.as_str()) == Some("adaptive");

    if is_adaptive {
        // 自适应思考没有 budget 概念，抱怨 budget 下限时改它反而报新错。
        // 但上游若是**不认 adaptive 这个形态**，就得给它一个显式预算 ——
        // 兜底注入改发 adaptive（与桌面端一致）之后，这条路才有可能被走到。
        return has_thinking && m.contains("adaptive");
    }

    let has_budget = m.contains("budget_tokens") || m.contains("budget tokens");
    let has_min = m.contains("greater than or equal to 1024")
        || m.contains(">= 1024")
        || (m.contains("1024") && m.contains("input should be"));
    has_budget && has_thinking && has_min
}

/// §3.11.2 动作：把 thinking 改成 enabled + 32000 budget；
/// budget 必须小于 max_tokens，所以 max_tokens 不够大时一并提到 64000。
pub(crate) fn apply_budget_fix(data: &mut serde_json::Value) -> bool {
    data["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": 32000});
    let max_tokens = data.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    if max_tokens < 32001 {
        data["max_tokens"] = serde_json::json!(64000);
    }
    true
}

/// §3.11.3 触发判定：历史消息里的 thinking / redacted_thinking block 与 signature
/// 是 Anthropic 官方的加密产物，第三方端点大多验不了或直接拒收。六种已知报法。
pub(crate) fn thinking_blocks_rectifier_applies(status: u16, msg: &str) -> bool {
    if !(400..500).contains(&status) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    let sig = m.contains("signature");
    (m.contains("invalid") && sig && m.contains("thinking") && m.contains("block"))
        || (m.contains("thought signature") && (m.contains("not valid") || m.contains("invalid")))
        || m.contains("must start with a thinking block")
        || (m.contains("expected")
            && (m.contains("thinking") || m.contains("redacted_thinking"))
            && m.contains("found")
            && m.contains("tool_use"))
        || (sig && m.contains("field required"))
        || (sig && m.contains("extra inputs are not permitted"))
}

/// 给这次 4xx 挑一个整流器（§3.11）。同类每请求只做一次，故传入已用过的列表。
///
/// 顺序即优先级：先试「错误消息明确点名」的，最后才是 effort 的投机分支 ——
/// budget 与 thinking 块的报错同样是 400，让投机分支先命中会白白浪费一次整流额度，
/// 而且删错了字段（去掉 effort 治不好 budget 下限）。
pub(crate) fn pick_rectifier(
    status: u16,
    msg: &str,
    data: &serde_json::Value,
    applied: &[Rectifier],
) -> Option<Rectifier> {
    let has_oc = data.get("output_config").is_some();
    let unused = |r: Rectifier| !applied.contains(&r);

    if unused(Rectifier::Effort) && effort_error_explicit(status, msg, has_oc) {
        Some(Rectifier::Effort)
    } else if unused(Rectifier::ThinkingBudget) && budget_rectifier_applies(status, msg, data) {
        Some(Rectifier::ThinkingBudget)
    } else if unused(Rectifier::ThinkingBlocks) && thinking_blocks_rectifier_applies(status, msg) {
        Some(Rectifier::ThinkingBlocks)
    } else if unused(Rectifier::Effort) && effort_error_speculative(status, has_oc) {
        Some(Rectifier::Effort)
    } else {
        None
    }
}

/// §3.11.3 的整流统计，写进请求日志。
#[derive(Default, Debug, PartialEq)]
pub(crate) struct StripStats {
    pub thinking: usize,
    pub redacted: usize,
    pub signatures: usize,
}

impl StripStats {
    pub(crate) fn is_empty(&self) -> bool {
        self.thinking == 0 && self.redacted == 0 && self.signatures == 0
    }
}

/// §3.11.3 动作：移除 messages 里全部 thinking / redacted_thinking block
/// 及其余块上的 signature 字段。块被删光的 assistant 消息整条移除 ——
/// 留一个空 content 数组多数端点同样会拒。
pub(crate) fn strip_thinking_blocks(data: &mut serde_json::Value) -> StripStats {
    let mut stats = StripStats::default();
    let Some(messages) = data.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return stats;
    };
    for msg in messages.iter_mut() {
        let Some(blocks) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue; // 纯字符串 content 不动
        };
        blocks.retain(|b| match b.get("type").and_then(|t| t.as_str()) {
            Some("thinking") => {
                stats.thinking += 1;
                false
            }
            Some("redacted_thinking") => {
                stats.redacted += 1;
                false
            }
            _ => true,
        });
        for b in blocks.iter_mut() {
            if let Some(obj) = b.as_object_mut() {
                if obj.remove("signature").is_some() {
                    stats.signatures += 1;
                }
            }
        }
    }
    // content 被删空的消息整条丢掉
    messages.retain(|m| {
        m.get("content")
            .and_then(|c| c.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(true)
    });
    stats
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

/// §3.3 未映射槽位的 400 响应体（Anthropic 错误格式，桌面端会直接渲染 message）。
pub(crate) fn unmapped_slot_body(slot: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "error": {
            "type": "invalid_request_error",
            "message": format!(
                "ModelLink: 模型槽位 {} 未映射到任何服务商。请在 ModelLink 中配置后重试。",
                slot
            )
        }
    })
}

/// 请求是不是流式的（§3.2：心跳只对流式响应有意义，非流式插注释行会污染 JSON）。
pub(crate) fn wants_stream(data: &serde_json::Value) -> bool {
    data.get("stream").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// SSE 心跳合流（§3.2）。上游沉默超过 `idle_secs` 就往下游写一行 SSE 注释
/// `: ping\n\n`，上游一有数据立刻重置计时；上游结束/出错则本流随之结束。
///
/// 为什么必须是本地代理来做：`inferenceStreamIdleTimeoutSec`（1.44121.1，300–1800）
/// 只在「网关往响应里写 keep-alive」时才生效 —— app.asar 原文：
/// *"A response on which nothing at all arrives — no pings — still fails after about
/// 5 minutes regardless of this key"*。ModelLink 正好站在中间，这是纯转发式工具做不到的。
///
/// 用注释行而不是伪造 `event: ping`：注释行是 SSE 规范里合法的保活手段，
/// 解析器会直接丢弃，绝不会被当成一个事件塞进消息流。
/// `idle_secs == 0` 表示关闭心跳。
pub(crate) fn with_heartbeat<S, E>(
    upstream: S,
    idle_secs: u64,
) -> tokio_stream::wrappers::ReceiverStream<Result<bytes::Bytes, E>>
where
    S: tokio_stream::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    use tokio_stream::StreamExt as _;

    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        tokio::pin!(upstream);
        loop {
            if idle_secs == 0 {
                match upstream.next().await {
                    Some(item) => {
                        if tx.send(item).await.is_err() {
                            return;
                        }
                    }
                    None => return,
                }
                continue;
            }
            let tick = tokio::time::sleep(std::time::Duration::from_secs(idle_secs));
            tokio::select! {
                item = upstream.next() => match item {
                    Some(item) => {
                        if tx.send(item).await.is_err() {
                            return; // 下游已断开
                        }
                    }
                    None => return, // 上游结束
                },
                _ = tick => {
                    if tx.send(Ok(bytes::Bytes::from_static(b": ping\n\n"))).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    tokio_stream::wrappers::ReceiverStream::new(rx)
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

    let resolved = match data.get("model").and_then(|m| m.as_str()) {
        Some(model) => match resolve_model(model, &config) {
            Ok(r) => {
                eprintln!("  model: {} -> {} ({})", model, r.model, r.target_url);
                data["model"] = serde_json::json!(r.model);
                // 身份说明换成这一次的真实模型：Chat 模式的系统提示词里没有当前模型 ID，
                // 只给全部映射的话模型不知道自己是哪一条（见 identity 模块）
                if crate::identity::personalize(&mut data, &r.model) {
                    eprintln!("  identity: 身份说明已换成 {}", r.model);
                }
                r
            }
            // §3.3：宁可报错也不静默换一个模型给用户
            Err(ResolveError::UnmappedSlot(slot)) => {
                eprintln!("  error: 槽位 {} 未映射到任何服务商", slot);
                push_log(
                    state.as_ref(),
                    LogEntry {
                        time: chrono_now(),
                        model: model.to_string(),
                        status: 400,
                        thinking: String::new(),
                        note: "未映射槽位".to_string(),
                        error: true,
                    },
                );
                return (StatusCode::BAD_REQUEST, Json(unmapped_slot_body(&slot))).into_response();
            }
        },
        None => ResolvedModel::default(),
    };

    // 标题生成优化（§5.5.4）+ thinking 三态注入（§3.10），顺序见 prepare_thinking 注释
    let (mut thinking_log, title_optimized) =
        prepare_thinking(&mut data, &resolved.thinking_effort);
    let mut notes: Vec<&'static str> = Vec::new();
    if title_optimized {
        eprintln!("  标题生成：已降到 effort=low + thinking disabled");
        notes.push("已优化：标题生成");
    }

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
    if state.thinking_blocks_rejected(&resolved.target_url, &resolved.model) {
        let st = strip_thinking_blocks(&mut data);
        if !st.is_empty() {
            eprintln!("  thinking 块: 上游已知不接受，本次先剥掉 {} 块", st.thinking + st.redacted);
            notes.push("thinking 块不支持（已缓存）");
        }
    }

    // 心跳只对流式响应有意义，且要在 data 被整流改写前定下来
    let streaming = wants_stream(&data);

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
                    // 整流后这一发成了：记住能力，后续请求直接不发（§3.11 副作用）
                    match r {
                        Rectifier::Effort => {
                            state.mark_effort_unsupported(&resolved.target_url, &resolved.model)
                        }
                        Rectifier::ThinkingBlocks => state
                            .mark_thinking_blocks_rejected(&resolved.target_url, &resolved.model),
                        // budget 下限是每请求的取值问题，不是服务商能力，不进缓存
                        Rectifier::ThinkingBudget => {}
                    }
                    notes.push(r.success_note());
                } else {
                    // 5xx：换了个失败方式，不足以断定上游不支持 —— 不写缓存，但要留痕
                    notes.push("已尝试自动修复");
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
            // §3.2：流式响应插 SSE 心跳，非流式保持原样直通（红线：字节等价）
            let body = if streaming {
                Body::from_stream(with_heartbeat(resp.bytes_stream(), HEARTBEAT_SECS))
            } else {
                Body::from_stream(resp.bytes_stream())
            };
            return (status, headers, body).into_response();
        }

        // 4xx：此时下游一个字节都还没写出去，所以「已吐出 SSE 事件的请求不重试」
        // 这条约束天然满足。缓冲错误体做整流判定。
        let headers = passthrough_headers(resp.headers());
        let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        let msg = upstream_error_message(&body);

        // 挑一个整流器（§3.11）。顺序即优先级：先试「错误消息明确点名」的，
        // 最后才是 effort 的投机分支 —— budget / thinking 块的报错同样是 400，
        // 让投机分支先命中会白白浪费一次整流额度。
        let pick = pick_rectifier(raw_status, &msg, &data, &applied);

        if applied.len() < MAX_RECTIFY {
            if let Some(r) = pick {
                let changed = match r {
                    Rectifier::Effort => {
                        let ok = strip_output_config(&mut data);
                        if ok {
                            thinking_log = String::new();
                        }
                        ok
                    }
                    Rectifier::ThinkingBudget => apply_budget_fix(&mut data),
                    Rectifier::ThinkingBlocks => {
                        let st = strip_thinking_blocks(&mut data);
                        if !st.is_empty() {
                            eprintln!(
                                "  rectify: 移除 {} 个 thinking 块 / {} 个 redacted 块 / {} 个签名",
                                st.thinking, st.redacted, st.signatures
                            );
                        }
                        !st.is_empty()
                    }
                };
                if changed {
                    eprintln!("  rectify: {}（上游 {}: {}）", r.attempt_note(), raw_status, msg);
                    applied.push(r);
                    first_error.get_or_insert((raw_status, headers, body));
                    continue;
                }
            }
        }

        // 无整流可做（或整流后仍失败）→ 原样返回最初的错误。
        break first_error.take().unwrap_or((raw_status, headers, body));
    };

    // 走到这里说明最终仍是 4xx：整流（若有）没能救回来。
    if let Some(r) = applied.last() {
        eprintln!("  rectify: 未生效，原样返回上游错误");
        notes.push(r.failed_note());
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
                    ModelEntry { name: "m-with-1m".into(), to_1m: "auto".into(), ..Default::default() },
                    ModelEntry { name: "m-plain".into(), to_1m: "".into(), ..Default::default() },
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
        assert_eq!(data[0]["id"], "claude-opus-5");
        assert_eq!(data[0]["display_name"], "m-with-1m");
        assert_eq!(data[0]["created"], 0);
        assert_eq!(data[1]["id"], "claude-opus-5[1m]");
        assert_eq!(data[1]["display_name"], "m-with-1m (1M)");
        assert_eq!(data[2]["id"], "claude-sonnet-5");
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
        // 请求完全没带 thinking 时，补的是**桌面端同款形态**：自适应。
        // 不写 budget_tokens —— 任何具体数字都是编的，而 adaptive 让上游自己决定。
        assert_eq!(data["thinking"], serde_json::json!({"type": "adaptive"}));

        let mut data = serde_json::json!({"model": "m"});
        assert_eq!(inject_thinking(&mut data, "max"), "max");
        assert_eq!(data["output_config"], serde_json::json!({"effort": "max"}));
    }

    #[test]
    fn fallback_injection_mirrors_what_the_desktop_actually_sends() {
        // §5.5.2 抓包：output_config={'effort': X} + thinking={'type': 'adaptive'}
        let mut data = serde_json::json!({"model": "m", "max_tokens": 64000});
        inject_thinking(&mut data, "xhigh");
        assert_eq!(
            data,
            serde_json::json!({
                "model": "m",
                "max_tokens": 64000,
                "output_config": {"effort": "xhigh"},
                "thinking": {"type": "adaptive"}
            })
        );
    }

    #[test]
    fn upstream_rejecting_adaptive_falls_back_to_an_explicit_budget() {
        // 兜底改发 adaptive 后多了一种可能的上游拒收；§3.11.2 顺带接住它
        let d = serde_json::json!({"thinking": {"type": "adaptive"}});
        assert!(budget_rectifier_applies(400, "thinking: adaptive is not supported", &d));
        let mut d = serde_json::json!({"max_tokens": 4096, "thinking": {"type": "adaptive"}});
        assert!(apply_budget_fix(&mut d));
        assert_eq!(d["thinking"], serde_json::json!({"type": "enabled", "budget_tokens": 32000}));
        // 但抱怨 budget 下限时仍然不该动 adaptive 请求（改了反而报新错）
        let d = serde_json::json!({"thinking": {"type": "adaptive"}});
        assert!(!budget_rectifier_applies(
            400,
            "thinking.budget_tokens: Input should be greater than or equal to 1024",
            &d
        ));
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

    fn with_oc() -> serde_json::Value {
        serde_json::json!({"output_config": {"effort": "max"}})
    }

    #[test]
    fn effort_rectifier_matches_explicit_error_messages() {
        for msg in [
            "output_config: extra inputs are not permitted",
            "unknown field `effort`",
            "Unsupported EFFORT level", // 大小写不敏感
        ] {
            assert_eq!(
                pick_rectifier(422, msg, &with_oc(), &[]),
                Some(Rectifier::Effort),
                "应命中: {msg}"
            );
        }
    }

    #[test]
    fn effort_rectifier_speculates_only_on_plain_400() {
        // 形态未知的 400 → 投机重试一次
        assert_eq!(
            pick_rectifier(400, "bad request", &with_oc(), &[]),
            Some(Rectifier::Effort)
        );
        // 与请求体无关的 4xx → 不动
        for (st, msg) in [(401, "invalid api key"), (429, "rate limited"), (404, "not found")] {
            assert_eq!(pick_rectifier(st, msg, &with_oc(), &[]), None, "{st} 不该整流");
        }
    }

    #[test]
    fn effort_rectifier_skips_when_nothing_to_strip_or_not_4xx() {
        assert_eq!(pick_rectifier(400, "effort bad", &serde_json::json!({}), &[]), None);
        assert_eq!(pick_rectifier(500, "effort bad", &with_oc(), &[]), None);
        assert_eq!(pick_rectifier(200, "effort bad", &with_oc(), &[]), None);
    }

    #[test]
    fn specific_rectifiers_win_over_the_speculative_effort_branch() {
        // 关键顺序：budget / thinking 块的报错也是 400 且请求里带着 output_config，
        // 若让 effort 的投机分支先命中，就会去删一个跟错误无关的字段。
        let mut d = with_oc();
        d["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": 100});
        assert_eq!(
            pick_rectifier(
                400,
                "thinking.budget_tokens: Input should be greater than or equal to 1024",
                &d,
                &[]
            ),
            Some(Rectifier::ThinkingBudget)
        );
        assert_eq!(
            pick_rectifier(400, "invalid signature on thinking block", &with_oc(), &[]),
            Some(Rectifier::ThinkingBlocks)
        );
    }

    #[test]
    fn each_rectifier_is_offered_at_most_once_per_request() {
        // §3.11 通用约束：同类只做一次，防循环
        assert_eq!(
            pick_rectifier(400, "bad request", &with_oc(), &[Rectifier::Effort]),
            None
        );
        let mut d = with_oc();
        d["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": 100});
        // budget 已经试过 → 退回 effort 的投机分支
        assert_eq!(
            pick_rectifier(400, "thinking budget_tokens >= 1024", &d, &[Rectifier::ThinkingBudget]),
            Some(Rectifier::Effort)
        );
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

    // ---- §3.2 SSE 心跳合流 ----

    use tokio_stream::StreamExt as _;

    /// 收集心跳流的产出，直到它结束。假时钟下 `tokio::time::advance` 推进时间。
    async fn drain(
        mut s: impl tokio_stream::Stream<Item = Result<bytes::Bytes, std::convert::Infallible>> + Unpin,
    ) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(item) = s.next().await {
            out.push(String::from_utf8_lossy(&item.unwrap()).to_string());
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn silent_upstream_gets_periodic_ping_comments() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::convert::Infallible>>(8);
        let stream = with_heartbeat(tokio_stream::wrappers::ReceiverStream::new(rx), 15);

        let handle = tokio::spawn(async move {
            // 上游沉默 50 秒后才吐一个字节
            tokio::time::sleep(std::time::Duration::from_secs(50)).await;
            tx.send(Ok(bytes::Bytes::from_static(b"data: hi\n\n"))).await.unwrap();
            drop(tx);
        });

        let got = drain(Box::pin(stream)).await;
        handle.await.unwrap();
        // 50 秒沉默 → 3 次心跳（15/30/45），随后是真数据
        assert_eq!(
            got,
            vec![": ping\n\n", ": ping\n\n", ": ping\n\n", "data: hi\n\n"],
        );
    }

    #[tokio::test(start_paused = true)]
    async fn upstream_data_resets_the_idle_timer() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::convert::Infallible>>(8);
        let stream = with_heartbeat(tokio_stream::wrappers::ReceiverStream::new(rx), 15);

        let handle = tokio::spawn(async move {
            // 每 10 秒来一次数据，永远不该触发心跳
            for i in 0..4 {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                tx.send(Ok(bytes::Bytes::from(format!("chunk{i}")))).await.unwrap();
            }
            drop(tx);
        });

        let got = drain(Box::pin(stream)).await;
        handle.await.unwrap();
        assert_eq!(got, vec!["chunk0", "chunk1", "chunk2", "chunk3"]);
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_stops_when_upstream_ends() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::convert::Infallible>>(8);
        let stream = with_heartbeat(tokio_stream::wrappers::ReceiverStream::new(rx), 15);
        drop(tx); // 上游立刻结束
        assert_eq!(drain(Box::pin(stream)).await, Vec::<String>::new());
    }

    #[tokio::test(start_paused = true)]
    async fn zero_interval_disables_the_heartbeat() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::convert::Infallible>>(8);
        let stream = with_heartbeat(tokio_stream::wrappers::ReceiverStream::new(rx), 0);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            tx.send(Ok(bytes::Bytes::from_static(b"x"))).await.unwrap();
            drop(tx);
        });
        let got = drain(Box::pin(stream)).await;
        handle.await.unwrap();
        assert_eq!(got, vec!["x"], "关掉心跳后 10 分钟沉默也不该有 ping");
    }

    #[test]
    fn heartbeat_only_applies_to_streaming_requests() {
        // 非流式响应整个 body 一次性到达，插心跳没有意义且会污染 JSON
        assert!(wants_stream(&serde_json::json!({"stream": true})));
        assert!(!wants_stream(&serde_json::json!({"stream": false})));
        assert!(!wants_stream(&serde_json::json!({})));
        assert!(!wants_stream(&serde_json::json!({"stream": "true"})));
    }

    // ---- §5.5.4 两个隐藏调用的识别 ----

    fn title_req() -> serde_json::Value {
        serde_json::json!({
            "model": "claude-opus-5",
            "max_tokens": 200,
            "messages": [{"role": "user", "content":
                "You are coming up with a succinct title for an agent conversation. …"}]
        })
    }

    #[test]
    fn title_generation_is_recognised_by_its_prompt_prefix() {
        assert!(is_title_generation(&title_req()));
        // 数组形态的 content 同样要认出来
        let mut d = title_req();
        d["messages"][0]["content"] = serde_json::json!([
            {"type": "text", "text": "You are coming up with a succinct title for an agent…"}
        ]);
        assert!(is_title_generation(&d));
    }

    #[test]
    fn ordinary_requests_are_not_mistaken_for_title_generation() {
        assert!(!is_title_generation(&serde_json::json!({
            "messages": [{"role": "user", "content": "帮我写个标题"}]
        })));
        // 只有首条 user 消息算数，别被后文里引用的同样文字骗了
        assert!(!is_title_generation(&serde_json::json!({
            "messages": [
                {"role": "user", "content": "随便聊聊"},
                {"role": "user", "content": "You are coming up with a succinct title for…"}
            ]
        })));
        assert!(!is_title_generation(&serde_json::json!({})));
    }

    #[test]
    fn title_generation_gets_the_cheapest_thinking_setting() {
        // 起个标题花 88 个思考 token 是纯浪费（§5.5.4 抓包）
        let mut d = title_req();
        assert!(optimize_title_generation(&mut d));
        assert_eq!(d["output_config"], serde_json::json!({"effort": "low"}));
        assert_eq!(d["thinking"], serde_json::json!({"type": "disabled"}));
    }

    #[test]
    fn title_optimization_respects_an_effort_the_desktop_already_set() {
        // 与 §3.10 同一条原则：桌面端明确指定了就不动
        let mut d = title_req();
        d["output_config"] = serde_json::json!({"effort": "max"});
        assert!(!optimize_title_generation(&mut d));
        assert_eq!(d["output_config"], serde_json::json!({"effort": "max"}));
    }

    // ---- 标题生成优化与服务商默认档的交互 ----

    #[test]
    fn title_optimization_is_not_blocked_by_the_providers_own_default() {
        // 服务商设了默认档（会被 inject_thinking 注入）时，标题优化仍应生效 ——
        // 「桌面端已选」这条豁免只该看**桌面端**发来的值，不该看 ModelLink 自己刚注入的
        let mut d = title_req();
        let (tag, optimized) = prepare_thinking(&mut d, "high");
        assert!(optimized, "服务商默认档不该挡住标题优化");
        assert_eq!(d["output_config"], serde_json::json!({"effort": "low"}));
        assert_eq!(d["thinking"], serde_json::json!({"type": "disabled"}));
        assert_eq!(tag, "low");
    }

    #[test]
    fn title_optimization_still_yields_to_a_desktop_chosen_effort() {
        let mut d = title_req();
        d["output_config"] = serde_json::json!({"effort": "max"});
        let (tag, optimized) = prepare_thinking(&mut d, "high");
        assert!(!optimized);
        assert_eq!(d["output_config"], serde_json::json!({"effort": "max"}));
        assert_eq!(tag, "max");
    }

    #[test]
    fn non_title_requests_go_through_the_normal_three_state_path() {
        let mut d = serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let (tag, optimized) = prepare_thinking(&mut d, "high");
        assert!(!optimized);
        assert_eq!(tag, "high");
        assert_eq!(d["output_config"], serde_json::json!({"effort": "high"}));
    }

    // ---- §3.3 未映射槽位的 400 响应体 ----

    #[test]
    fn unmapped_slot_body_is_an_anthropic_error() {
        let v = unmapped_slot_body("claude-opus-5");
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "invalid_request_error");
        assert_eq!(
            v["error"]["message"],
            "ModelLink: 模型槽位 claude-opus-5 未映射到任何服务商。请在 ModelLink 中配置后重试。"
        );
    }

    // ---- §3.11.2 thinking budget 约束 ----

    #[test]
    fn budget_rectifier_matches_the_1024_lower_bound_complaints() {
        let d = serde_json::json!({"thinking": {"type": "enabled", "budget_tokens": 100}});
        for msg in [
            "thinking.budget_tokens: Input should be greater than or equal to 1024",
            "invalid thinking budget tokens, must be >= 1024",
            "thinking.budget_tokens: input should be 1024 or more",
        ] {
            assert!(budget_rectifier_applies(400, msg, &d), "应命中: {msg}");
        }
    }

    #[test]
    fn budget_rectifier_needs_all_three_signals() {
        let d = serde_json::json!({"thinking": {"type": "enabled", "budget_tokens": 100}});
        // 缺 thinking
        assert!(!budget_rectifier_applies(400, "budget_tokens must be >= 1024", &d));
        // 缺 1024 下限约束
        assert!(!budget_rectifier_applies(400, "thinking.budget_tokens is wrong", &d));
        // 缺 budget 字样
        assert!(!budget_rectifier_applies(400, "thinking must be >= 1024", &d));
        // 非 4xx
        assert!(!budget_rectifier_applies(500, "thinking.budget_tokens >= 1024", &d));
    }

    #[test]
    fn budget_rectifier_never_touches_adaptive_thinking() {
        // 自适应思考没有 budget 概念，改了反而报新错
        let d = serde_json::json!({"thinking": {"type": "adaptive"}});
        assert!(!budget_rectifier_applies(
            400,
            "thinking.budget_tokens: Input should be greater than or equal to 1024",
            &d
        ));
    }

    #[test]
    fn budget_fix_raises_budget_and_max_tokens_together() {
        let mut d = serde_json::json!({
            "max_tokens": 4096,
            "thinking": {"type": "enabled", "budget_tokens": 100}
        });
        assert!(apply_budget_fix(&mut d));
        assert_eq!(d["thinking"], serde_json::json!({"type": "enabled", "budget_tokens": 32000}));
        // budget 必须小于 max_tokens，否则换来一个新错误
        assert_eq!(d["max_tokens"], 64000);
    }

    #[test]
    fn budget_fix_leaves_a_large_enough_max_tokens_alone() {
        let mut d = serde_json::json!({"max_tokens": 64000, "thinking": {"type": "disabled"}});
        assert!(apply_budget_fix(&mut d));
        assert_eq!(d["thinking"]["budget_tokens"], 32000);
        assert_eq!(d["thinking"]["type"], "enabled");
        assert_eq!(d["max_tokens"], 64000, "够大就别动用户的设置");
    }

    // ---- §3.11.3 thinking 块结构 / 签名 ----

    #[test]
    fn thinking_block_rectifier_matches_every_documented_shape() {
        for msg in [
            "invalid signature on thinking block",
            "thought signature is not valid",
            "The thought signature was invalid",
            "messages.1: must start with a thinking block",
            "expected thinking or redacted_thinking but found tool_use",
            "messages.1.content.0.signature: field required",
            "messages.1.content.0.signature: extra inputs are not permitted",
        ] {
            assert!(thinking_blocks_rectifier_applies(400, msg), "应命中: {msg}");
        }
    }

    #[test]
    fn thinking_block_rectifier_ignores_unrelated_errors() {
        assert!(!thinking_blocks_rectifier_applies(400, "invalid api key"));
        assert!(!thinking_blocks_rectifier_applies(400, "signature"));
        assert!(!thinking_blocks_rectifier_applies(500, "invalid signature thinking block"));
    }

    #[test]
    fn stripping_removes_thinking_blocks_and_signatures_with_counts() {
        let mut d = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "嗯…", "signature": "abc"},
                    {"type": "redacted_thinking", "data": "xxx"},
                    {"type": "text", "text": "答案"},
                    {"type": "tool_use", "id": "t1", "name": "f", "input": {}, "signature": "def"}
                ]},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "再想想", "signature": "ghi"}
                ]}
            ]
        });
        let stats = strip_thinking_blocks(&mut d);
        assert_eq!((stats.thinking, stats.redacted, stats.signatures), (2, 1, 1));
        // 字符串 content 不动
        assert_eq!(d["messages"][0]["content"], "hi");
        // 只剩 text 与 tool_use，且 tool_use 上的签名被摘掉
        let kept = d["messages"][1]["content"].as_array().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0]["type"], "text");
        assert_eq!(kept[1]["type"], "tool_use");
        assert!(kept[1].get("signature").is_none());
        // 整条消息的块被删光后，不能留一个空 content 数组（多数端点会拒）
        assert_eq!(d["messages"].as_array().unwrap().len(), 2, "空 assistant 消息应被移除");
    }

    #[test]
    fn stripping_reports_nothing_when_there_is_nothing_to_strip() {
        let mut d = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        let stats = strip_thinking_blocks(&mut d);
        assert!(stats.is_empty());
    }

    // ---- §3.11.4 能力缓存（按服务商 + 真实上游模型 ID，绝不按槽位名） ----

    #[test]
    fn caps_cache_treats_the_1m_variant_as_the_same_model() {
        // [1m] 只是路由后缀，背后是同一个上游模型 —— 学到的能力必须共用
        let state = ProxyState::new(Config::default()).unwrap();
        state.mark_effort_unsupported("https://a.example.com", "real-a[1m]");
        assert!(state.effort_unsupported("https://a.example.com", "real-a"));
        assert!(state.effort_unsupported("https://a.example.com", "real-a[1m]"));

        state.mark_thinking_blocks_rejected("https://a.example.com", "real-b");
        assert!(state.thinking_blocks_rejected("https://a.example.com", "real-b[1m]"));
    }

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
