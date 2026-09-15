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
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::net::TcpListener;

use crate::config::{
    flatten_config, resolve_model, Config, ModelPricing, ResolveError, ResolvedModel,
    HEARTBEAT_SECS,
};

pub const MAX_LOGS: usize = 100;

/// 单请求最多整流次数（§3.11 通用约束，防循环）。同类整流每请求只做一次。
pub(crate) const MAX_RECTIFY: usize = 2;

#[derive(Serialize, Clone, Default)]
pub struct LogEntry {
    /// 2.2：自增编号。流式响应要等传完才知道耗时和用量，按它回填。
    pub id: u64,
    pub time: String,
    /// 2.2：Claude 请求的槽位名（原样，可能带 `[1m]`）。日志页据此找到对应的服务商。
    pub slot: String,
    pub model: String,
    pub status: u16,
    /// 本次实际发给上游的推理强度（""=未发 / "off" / low…max）。
    pub thinking: String,
    /// 2.1-A 附注：整流标记、未映射槽位等。多条以 `NOTE_SEPARATOR` 连接，空 = 无附注。
    pub note: String,
    /// 2.1-A：ModelLink 自己判定为错误的请求（日志页标红）。
    pub error: bool,
    /// 2.2：耗时（毫秒）：收到请求 → 响应传完。None = 还在传。
    pub duration_ms: Option<u64>,
    /// 2.2：上游 usage 里的 token 数。上游没给、或响应中途断开时为 None。
    pub usage: Option<Usage>,
    /// 2.2：按这个模型的费率算出的花费（USD）。没有费率或没有 usage 时为 None —— 绝不估算。
    pub cost_usd: Option<f64>,
    /// 2.2：出错时上游给的说明（`error.message`，截断）。
    pub detail: String,
}

/// 2.2：上游 usage 里的 token 数（Anthropic 格式）。
#[derive(Serialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    /// 并入一个 usage 对象，各项取大 —— 流式响应分几次给（message_start 给输入、
    /// message_delta 给最终输出），数字只增不减；有的上游在 message_delta 里把输入写成 0。
    /// 返回这个对象里有没有认得的字段。
    pub(crate) fn merge(&mut self, v: &serde_json::Value) -> bool {
        let mut seen = false;
        let mut take = |key: &str, slot: &mut u64| {
            if let Some(n) = v.get(key).and_then(|n| n.as_u64()) {
                *slot = (*slot).max(n);
                seen = true;
            }
        };
        take("input_tokens", &mut self.input_tokens);
        take("output_tokens", &mut self.output_tokens);
        take("cache_read_input_tokens", &mut self.cache_read_tokens);
        take("cache_creation_input_tokens", &mut self.cache_write_tokens);
        seen
    }

    /// 按费率算花费（USD）。输入 / 输出价缺一个就不算 —— 那两个数编不得；
    /// 缓存价没有按 0 计（与写进 Claude 的费率表同一口径，见 gateway::inference_model_pricing_entries）。
    pub(crate) fn cost_usd(&self, p: &ModelPricing) -> Option<f64> {
        let (input, output) = (p.input?, p.output?);
        let per_token = |n: u64, price: f64| n as f64 * price / 1_000_000.0;
        Some(
            per_token(self.input_tokens, input)
                + per_token(self.output_tokens, output)
                + per_token(self.cache_read_tokens, p.cache_read.unwrap_or(0.0))
                + per_token(self.cache_write_tokens, p.cache_write.unwrap_or(0.0)),
        )
    }
}

/// 2.2：今天的请求统计（日志页顶部摘要）。
///
/// 不从那 100 条日志里现算 —— 一个 Claude Code 会话就能刷掉几十条，
/// 「今日」要是只算留下来的那部分，数字就是错的。
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct TodayStats {
    /// 从这个时刻起算（Unix 秒）：本地零点，或 ModelLink 今天启动的时刻，取晚的
    pub since: u64,
    /// 已经结束的请求数
    pub requests: u64,
    pub failures: u64,
    pub duration_total_ms: u64,
    pub duration_max_ms: u64,
    /// 输入侧合计（含缓存命中与缓存写入）
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 拿到 usage 的请求数；少于 requests 说明 token 合计不全
    pub with_usage: u64,
    pub cost_usd: f64,
    /// 算得出花费的请求数；少于 with_usage 说明有模型没有费率，花费合计不能用
    pub priced: u64,
    #[serde(skip)]
    day: i64,
}

impl TodayStats {
    /// 换成 `now` 所在的那一天（跨零点时清零）。
    fn roll(&mut self, now: u64, started_at: u64) {
        let day = local_day(now);
        if day != self.day {
            *self = TodayStats { day, since: local_midnight(day).max(started_at), ..Default::default() };
        }
    }

    fn record(&mut self, status: u16, error: bool, duration_ms: u64, usage: Option<Usage>, cost: Option<f64>) {
        self.requests += 1;
        if error || status >= 400 {
            self.failures += 1;
        }
        self.duration_total_ms += duration_ms;
        self.duration_max_ms = self.duration_max_ms.max(duration_ms);
        if let Some(u) = usage {
            self.with_usage += 1;
            self.input_tokens += u.input_tokens + u.cache_read_tokens + u.cache_write_tokens;
            self.output_tokens += u.output_tokens;
            if let Some(c) = cost {
                self.priced += 1;
                self.cost_usd += c;
            }
        }
    }
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
    /// 2.2：下一条日志的编号。
    next_log_id: AtomicU64,
    /// 2.2：今天的请求统计。
    today: Mutex<TodayStats>,
    /// 2.2：进程启动时刻（Unix 秒）—— 今天中途启动时，「今日」从这里算起。
    started_at: u64,
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
            next_log_id: AtomicU64::new(1),
            today: Mutex::new(TodayStats::default()),
            started_at: unix_now(),
        })
    }

    /// 记一条日志，返回它的编号（流式响应传完后按编号回填）。
    pub fn push_log(&self, mut entry: LogEntry) -> u64 {
        entry.id = self.next_log_id.fetch_add(1, Ordering::SeqCst);
        let id = entry.id;
        let mut logs = self.logs.write().unwrap_or_else(|e| e.into_inner());
        logs.push(entry);
        let len = logs.len();
        if len > MAX_LOGS {
            logs.drain(0..len - MAX_LOGS);
        }
        id
    }

    /// 请求结束：回填耗时 / 用量 / 花费 / 错误说明，并计入今日统计。
    /// 日志条目可能已经被新请求挤出 100 条，统计照记。
    pub fn finish_log(&self, id: u64, finished: Finished) {
        let Finished { status, error, duration_ms, usage, cost_usd, detail } = finished;
        {
            let mut logs = self.logs.write().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = logs.iter_mut().rev().find(|e| e.id == id) {
                e.duration_ms = Some(duration_ms);
                e.usage = usage;
                e.cost_usd = cost_usd;
                if !detail.is_empty() {
                    e.detail = detail;
                }
            }
        }
        let mut today = self.today.lock().unwrap_or_else(|e| e.into_inner());
        today.roll(unix_now(), self.started_at);
        today.record(status, error, duration_ms, usage, cost_usd);
    }

    /// 今天的请求统计（没有请求时也会按日期翻篇）。
    pub fn today_stats(&self) -> TodayStats {
        let mut today = self.today.lock().unwrap_or_else(|e| e.into_inner());
        today.roll(unix_now(), self.started_at);
        today.clone()
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

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 本地时区相对 UTC 的偏移（秒）。非 macOS 沿用 v1 的固定东八区。
fn local_offset_secs() -> i64 {
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
}

/// 本地日期序号（自 1970-01-01 起的第几天，按本地时区）。
fn local_day(ts: u64) -> i64 {
    (ts as i64 + local_offset_secs()).div_euclid(86400)
}

/// 某个本地日期序号的零点（Unix 秒）。
fn local_midnight(day: i64) -> u64 {
    (day * 86400 - local_offset_secs()).max(0) as u64
}

pub fn chrono_now() -> String {
    let d = unix_now();
    let offset_secs = local_offset_secs();
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

/// Auto 模式安全检查的特征串：桌面端自带的 Claude Code（2.1.270 抓包）发检查请求时，系统提示词这样开头。
const AUTO_MODE_CHECK_PREFIX: &str = "You are a security monitor for autonomous AI coding agents";

/// 日志里给安全检查请求的标记。
pub(crate) const NOTE_AUTO_MODE_CHECK: &str = "Auto 模式安全检查";

/// 这是不是 Auto 模式的安全检查：每一步有风险的操作前，引擎另发一次请求让模型判断能不能做。
///
/// 检查优先发给 claude-sonnet-5，它没映射或出错才改用对话自己的模型 —— 映射到别家服务商时，
/// 这些请求花的是那一家的钱，用户在 Claude 里完全看不到。只在日志里标出来，不改请求；
/// 文案哪天改了认不出，也只是少一个标记。
pub(crate) fn is_auto_mode_check(data: &serde_json::Value) -> bool {
    let starts = |t: &str| t.trim_start().starts_with(AUTO_MODE_CHECK_PREFIX);
    match data.get("system") {
        Some(serde_json::Value::String(s)) => starts(s),
        Some(serde_json::Value::Array(blocks)) => {
            blocks.iter().any(|b| b.get("text").and_then(|t| t.as_str()).is_some_and(starts))
        }
        _ => false,
    }
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

/// 一条日志有多个标记时用它连接。标记文字里自带「 · 」（如「已整流 · 思考预算过小」），不能拿它当分隔。
const NOTE_SEPARATOR: &str = "；";

/// 日志标记：上游不认 `output_config.effort`。现场整流成功、和能力缓存命中直接不发，用的是同一句。
const NOTE_EFFORT: &str = "已整流 · 上游不认推理档位";
/// 日志标记：上游收不了历史 thinking 块（同上，两条路径同一句）。
const NOTE_THINKING_BLOCKS: &str = "已整流 · 上游不认思考块签名";

/// 日志里「上游的错误说明」最多留多少字 —— 够看懂，又不至于把整页撑爆。
const DETAIL_MAX_CHARS: usize = 300;

fn truncate_detail(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= DETAIL_MAX_CHARS {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(DETAIL_MAX_CHARS).collect::<String>())
    }
}

/// 请求结束时回填日志的内容。
pub struct Finished {
    pub status: u16,
    pub error: bool,
    pub duration_ms: u64,
    pub usage: Option<Usage>,
    pub cost_usd: Option<f64>,
    pub detail: String,
}

/// 响应体旁路扫描：一个字节都不改，只从里面找 usage 和错误说明（§6.3 日志的耗时 / token / 花费）。
///
/// SSE 按行扫 `data:`；普通 JSON 响应攒起来等结束时解析（有上限，超了就不算用量）。
pub(crate) struct ResponseScan {
    sse: bool,
    /// 普通 JSON 响应超过上限：不再攒，也不给 usage
    overflowed: bool,
    buf: Vec<u8>,
    usage: Usage,
    saw_usage: bool,
    /// SSE 看到了 message_stop / JSON 完整解析 —— 没走完的响应不给 usage，半截的 token 数会让花费偏低
    complete: bool,
    detail: String,
}

/// 普通 JSON 响应最多攒这么多字节来找 usage。
const SCAN_JSON_LIMIT: usize = 8 * 1024 * 1024;
/// SSE 单行最长留这么多（正常的事件行远小于它）。
const SCAN_LINE_LIMIT: usize = 1024 * 1024;

impl ResponseScan {
    pub(crate) fn new(sse: bool) -> Self {
        Self {
            sse,
            overflowed: false,
            buf: Vec::new(),
            usage: Usage::default(),
            saw_usage: false,
            complete: false,
            detail: String::new(),
        }
    }

    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        if !self.sse {
            if self.overflowed {
                return;
            }
            if self.buf.len() + chunk.len() <= SCAN_JSON_LIMIT {
                self.buf.extend_from_slice(chunk);
            } else {
                self.overflowed = true;
                self.buf = Vec::new();
            }
            return;
        }
        self.buf.extend_from_slice(chunk);
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            self.scan_line(&line);
        }
        if self.buf.len() > SCAN_LINE_LIMIT {
            self.buf.clear();
        }
    }

    fn scan_line(&mut self, line: &[u8]) {
        let Some(rest) = line.strip_prefix(b"data:") else { return };
        // 内容增量占了绝大多数行，先按字节粗筛，省得每行都解析一遍 JSON
        let wanted = [&b"usage"[..], b"message_stop", b"error"];
        if !wanted.iter().any(|w| rest.windows(w.len()).any(|win| win == *w)) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(rest.trim_ascii()) else { return };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("message_start") => {
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.saw_usage |= self.usage.merge(u);
                }
            }
            Some("message_delta") => {
                if let Some(u) = v.get("usage") {
                    self.saw_usage |= self.usage.merge(u);
                }
            }
            Some("message_stop") => self.complete = true,
            Some("error") => {
                if let Some(m) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                    self.detail = truncate_detail(m);
                }
            }
            _ => {}
        }
    }

    /// 收尾：返回（usage，错误说明）。
    pub(crate) fn finish(&mut self, status: u16) -> (Option<Usage>, String) {
        if !self.sse && !self.buf.is_empty() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&self.buf) {
                self.complete = true;
                if let Some(u) = v.get("usage") {
                    self.saw_usage |= self.usage.merge(u);
                }
            }
            if status >= 400 {
                self.detail = truncate_detail(&upstream_error_message(&self.buf));
            }
        }
        let usage = (self.complete && self.saw_usage).then_some(self.usage);
        (usage, std::mem::take(&mut self.detail))
    }
}

/// 挂在转发流上：流传完（或下游中途断开、流被丢弃）时回填这条日志。
pub(crate) struct LogFinisher {
    pub(crate) state: Arc<ProxyState>,
    pub(crate) id: u64,
    pub(crate) status: u16,
    pub(crate) started: std::time::Instant,
    pub(crate) pricing: Option<ModelPricing>,
    pub(crate) scan: ResponseScan,
}

impl Drop for LogFinisher {
    fn drop(&mut self) {
        let (usage, detail) = self.scan.finish(self.status);
        let cost_usd = usage.and_then(|u| self.pricing.as_ref().and_then(|p| u.cost_usd(p)));
        self.state.finish_log(
            self.id,
            Finished {
                status: self.status,
                error: false,
                duration_ms: self.started.elapsed().as_millis() as u64,
                usage,
                cost_usd,
                detail,
            },
        );
    }
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
    /// 整流成功后写进请求日志的标记（日志页原样显示，写给用户看）。
    pub(crate) fn success_note(self) -> &'static str {
        match self {
            Rectifier::Effort => NOTE_EFFORT,
            Rectifier::ThinkingBudget => "已整流 · 思考预算过小",
            Rectifier::ThinkingBlocks => NOTE_THINKING_BLOCKS,
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
            Rectifier::Effort => "整流未生效 · 推理档位",
            Rectifier::ThinkingBudget => "整流未生效 · 思考预算",
            Rectifier::ThinkingBlocks => "整流未生效 · 思考块签名",
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

async fn proxy_fallback(
    State(state): State<Arc<ProxyState>>,
    req: axum::http::Request<Body>,
) -> axum::response::Response {
    use tokio_stream::StreamExt as _;

    let started = std::time::Instant::now();
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

    // Claude 请求的槽位名，原样记进日志（`data["model"]` 马上会被换成上游模型名）
    let requested = data.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();

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
                let note = if is_auto_mode_check(&data) {
                    [NOTE_AUTO_MODE_CHECK, "未映射槽位"].join(NOTE_SEPARATOR)
                } else {
                    "未映射槽位".to_string()
                };
                let id = state.push_log(LogEntry {
                    time: chrono_now(),
                    slot: model.to_string(),
                    model: model.to_string(),
                    status: 400,
                    note,
                    error: true,
                    ..Default::default()
                });
                state.finish_log(
                    id,
                    Finished {
                        status: 400,
                        error: true,
                        duration_ms: started.elapsed().as_millis() as u64,
                        usage: None,
                        cost_usd: None,
                        detail: String::new(),
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
    if is_auto_mode_check(&data) {
        eprintln!("  Auto 模式安全检查");
        notes.push(NOTE_AUTO_MODE_CHECK);
    }
    if title_optimized {
        eprintln!("  标题生成：已降到 effort=low + thinking disabled");
        notes.push("标题生成 · 已省思考");
    }

    if resolved.target_url.is_empty() {
        eprintln!("  error: no target URL configured for this model");
        // 请求里连模型名都没有的（不是 Claude 发的）不记，免得日志里出现一条没头没尾的错误
        if !requested.is_empty() {
            let id = state.push_log(LogEntry {
                time: chrono_now(),
                slot: requested.clone(),
                model: resolved.model.clone(),
                status: 502,
                thinking: thinking_log.clone(),
                note: "服务商没填 API 地址".to_string(),
                error: true,
                ..Default::default()
            });
            state.finish_log(
                id,
                Finished {
                    status: 502,
                    error: true,
                    duration_ms: started.elapsed().as_millis() as u64,
                    usage: None,
                    cost_usd: None,
                    detail: String::new(),
                },
            );
        }
        return (StatusCode::BAD_GATEWAY, "No API URL configured for this model. Please configure the provider in the proxy app.").into_response();
    }

    // 能力缓存命中：这家上游已知不认 output_config.effort，直接不发，
    // 省掉每次的失败往返（§3.11.1 副作用）。
    if state.effort_unsupported(&resolved.target_url, &resolved.model)
        && strip_output_config(&mut data)
    {
        eprintln!("  effort: 上游已知不支持，本次不发送");
        notes.push(NOTE_EFFORT);
        thinking_log = String::new();
    }
    if state.thinking_blocks_rejected(&resolved.target_url, &resolved.model) {
        let st = strip_thinking_blocks(&mut data);
        if !st.is_empty() {
            eprintln!("  thinking 块: 上游已知不接受，本次先剥掉 {} 块", st.thinking + st.redacted);
            notes.push(NOTE_THINKING_BLOCKS);
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
                let mut note = notes.clone();
                note.push("连不上服务商");
                let id = state.push_log(LogEntry {
                    time: chrono_now(),
                    slot: requested.clone(),
                    model: data.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                    status: 502,
                    thinking: thinking_log.clone(),
                    note: note.join(NOTE_SEPARATOR),
                    error: true,
                    ..Default::default()
                });
                state.finish_log(
                    id,
                    Finished {
                        status: 502,
                        error: true,
                        duration_ms: started.elapsed().as_millis() as u64,
                        usage: None,
                        cost_usd: None,
                        detail: truncate_detail(&e.to_string()),
                    },
                );
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
                    notes.push("整流后上游仍出错");
                }
            }
            let status =
                StatusCode::from_u16(raw_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let headers = passthrough_headers(resp.headers());
            let sse = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream"));
            let log_id = data.get("model").and_then(|m| m.as_str()).map(|model| {
                state.push_log(LogEntry {
                    time: chrono_now(),
                    slot: requested.clone(),
                    model: model.to_string(),
                    status: raw_status,
                    thinking: thinking_log.clone(),
                    note: notes.join(NOTE_SEPARATOR),
                    error: false,
                    ..Default::default()
                })
            });
            // 旁路扫描响应里的 usage（一个字节都不改），流传完或下游断开时回填日志。
            // §3.2：流式响应插 SSE 心跳，非流式保持原样直通（红线：字节等价）
            let mut finisher = log_id.map(|id| LogFinisher {
                state: state.clone(),
                id,
                status: raw_status,
                started,
                pricing: resolved.pricing.clone(),
                scan: ResponseScan::new(sse),
            });
            let upstream = resp.bytes_stream().map(move |item| {
                if let (Some(f), Ok(chunk)) = (finisher.as_mut(), &item) {
                    f.scan.feed(chunk);
                }
                item
            });
            let body = if streaming {
                Body::from_stream(with_heartbeat(upstream, HEARTBEAT_SECS))
            } else {
                Body::from_stream(upstream)
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
        let id = state.push_log(LogEntry {
            time: chrono_now(),
            slot: requested.clone(),
            model: model.to_string(),
            status: raw_status,
            thinking: thinking_log.clone(),
            note: notes.join(NOTE_SEPARATOR),
            error: false,
            ..Default::default()
        });
        state.finish_log(
            id,
            Finished {
                status: raw_status,
                error: false,
                duration_ms: started.elapsed().as_millis() as u64,
                usage: None,
                cost_usd: None,
                detail: truncate_detail(&upstream_error_message(&err_body)),
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

    // ---- 2.2 请求日志：耗时 / token / 花费 ----

    #[test]
    fn usage_merge_keeps_the_larger_number_per_field() {
        // 流式响应分两次给：message_start 给输入，message_delta 给最终输出（有的上游把输入写成 0）
        let mut u = Usage::default();
        assert!(u.merge(&serde_json::json!({
            "input_tokens": 1200, "output_tokens": 1,
            "cache_read_input_tokens": 300, "cache_creation_input_tokens": 40
        })));
        assert!(u.merge(&serde_json::json!({"input_tokens": 0, "output_tokens": 856})));
        assert_eq!(u, Usage { input_tokens: 1200, output_tokens: 856, cache_read_tokens: 300, cache_write_tokens: 40 });
        assert!(!u.merge(&serde_json::json!({"something": 1})), "认不出的对象不算有 usage");
    }

    #[test]
    fn cost_needs_both_input_and_output_prices() {
        let u = Usage { input_tokens: 1_000_000, output_tokens: 500_000, cache_read_tokens: 2_000_000, cache_write_tokens: 0 };
        let full = ModelPricing { input: Some(0.5), output: Some(2.0), cache_read: Some(0.1), cache_write: None };
        // 0.5 + 1.0 + 0.2；缓存写入没有价按 0
        assert!((u.cost_usd(&full).unwrap() - 1.7).abs() < 1e-9);
        // 缺输出价就不算 —— 宁可不显示，也不给假账单
        let half = ModelPricing { input: Some(0.5), ..Default::default() };
        assert_eq!(u.cost_usd(&half), None);
    }

    fn sse(events: &[serde_json::Value]) -> String {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e))
            .collect()
    }

    #[test]
    fn sse_scan_finds_usage_even_when_split_mid_line() {
        let body = sse(&[
            serde_json::json!({"type": "message_start", "message": {"usage": {"input_tokens": 12, "cache_read_input_tokens": 5, "output_tokens": 1}}}),
            serde_json::json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "hello usage"}}),
            serde_json::json!({"type": "message_delta", "usage": {"output_tokens": 34}}),
            serde_json::json!({"type": "message_stop"}),
        ]);
        // 按 7 字节切，保证每行都被切断过
        let mut scan = ResponseScan::new(true);
        for chunk in body.as_bytes().chunks(7) {
            scan.feed(chunk);
        }
        let (usage, detail) = scan.finish(200);
        assert_eq!(usage, Some(Usage { input_tokens: 12, output_tokens: 34, cache_read_tokens: 5, cache_write_tokens: 0 }));
        assert_eq!(detail, "");
    }

    #[test]
    fn interrupted_stream_reports_no_usage() {
        // 用户在 Claude 里点了停止：没有 message_stop，输出 token 数不全 —— 不给数，免得花费偏低
        let body = sse(&[
            serde_json::json!({"type": "message_start", "message": {"usage": {"input_tokens": 12}}}),
        ]);
        let mut scan = ResponseScan::new(true);
        scan.feed(body.as_bytes());
        assert_eq!(scan.finish(200).0, None);
    }

    #[test]
    fn sse_error_event_becomes_the_detail() {
        let body = sse(&[serde_json::json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})]);
        let mut scan = ResponseScan::new(true);
        scan.feed(body.as_bytes());
        assert_eq!(scan.finish(200).1, "Overloaded");
    }

    #[test]
    fn json_scan_reads_usage_and_error_message() {
        let ok = br#"{"type":"message","content":[],"usage":{"input_tokens":7,"output_tokens":3}}"#;
        let mut scan = ResponseScan::new(false);
        scan.feed(&ok[..10]);
        scan.feed(&ok[10..]);
        assert_eq!(scan.finish(200).0, Some(Usage { input_tokens: 7, output_tokens: 3, ..Default::default() }));

        let err = br#"{"type":"error","error":{"type":"api_error","message":"upstream exploded"}}"#;
        let mut scan = ResponseScan::new(false);
        scan.feed(err);
        assert_eq!(scan.finish(503), (None, "upstream exploded".to_string()));
    }

    #[test]
    fn oversized_json_response_is_not_scanned() {
        let mut scan = ResponseScan::new(false);
        scan.feed(br#"{"usage":{"input_tokens":1,"output_tokens":1},"pad":""#);
        scan.feed(&vec![b'x'; SCAN_JSON_LIMIT]);
        scan.feed(br#""}"#);
        assert_eq!(scan.finish(200).0, None);
    }

    #[test]
    fn today_stats_count_everything_and_roll_over_at_midnight() {
        let now = 1_800_000_000u64;
        let mut t = TodayStats::default();
        t.roll(now, 0);
        let u = Usage { input_tokens: 100, output_tokens: 20, cache_read_tokens: 50, cache_write_tokens: 0 };
        t.record(200, false, 1200, Some(u), Some(0.01));
        t.record(200, false, 3000, Some(u), None); // 这个模型没费率
        t.record(502, true, 40, None, None);
        assert_eq!((t.requests, t.failures, t.with_usage, t.priced), (3, 1, 2, 1));
        assert_eq!((t.input_tokens, t.output_tokens), (300, 40), "输入侧含缓存命中");
        assert_eq!((t.duration_total_ms, t.duration_max_ms), (4240, 3000));
        assert!(t.since <= now);

        t.roll(now + 60, 0);
        assert_eq!(t.requests, 3, "同一天不清零");
        t.roll(now + 86_400, 0);
        assert_eq!(t.requests, 0, "跨天清零");
        // ModelLink 今天中途才启动：从启动时刻算起
        let mut t = TodayStats::default();
        t.roll(now, now - 5);
        assert_eq!(t.since, now - 5);
    }

    #[test]
    fn finish_log_fills_the_entry_and_counts_even_after_eviction() {
        let state = ProxyState::new(Config::default()).unwrap();
        let done = |status| Finished {
            status,
            error: false,
            duration_ms: 900,
            usage: Some(Usage { input_tokens: 10, output_tokens: 2, ..Default::default() }),
            cost_usd: Some(0.5),
            detail: String::new(),
        };
        let first = state.push_log(LogEntry { model: "m".into(), status: 200, ..Default::default() });
        {
            let logs = state.logs.read().unwrap();
            assert_eq!(logs[0].duration_ms, None, "传完之前没有耗时");
            assert_eq!(logs[0].id, first);
        }
        state.finish_log(first, done(200));
        assert_eq!(state.logs.read().unwrap()[0].duration_ms, Some(900));
        assert_eq!(state.logs.read().unwrap()[0].cost_usd, Some(0.5));

        // 一个长请求还没传完，期间来了 100 条新请求把它挤出去 —— 统计照记
        let long = state.push_log(LogEntry { model: "long".into(), status: 200, ..Default::default() });
        for _ in 0..MAX_LOGS {
            state.push_log(LogEntry::default());
        }
        state.finish_log(long, done(200));
        assert!(state.logs.read().unwrap().iter().all(|e| e.id != long));
        assert_eq!(state.today_stats().requests, 2);
    }

    /// 起一个假上游 + 一个真代理，返回（代理状态，代理地址）。
    async fn proxy_with_upstream(upstream: Router, pricing: Option<ModelPricing>) -> (Arc<ProxyState>, String) {
        let ul = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_url = format!("http://{}", ul.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(ul, upstream).await.unwrap() });
        let cfg = Config {
            providers: vec![Provider {
                target_url: upstream_url,
                api_key: "k".into(),
                models: vec![ModelEntry { name: "real-model".into(), pricing_synced: pricing, ..Default::default() }],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let state = Arc::new(no_proxy_state(cfg));
        let pl = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("http://{}", pl.local_addr().unwrap());
        tokio::spawn(serve(pl, state.clone()));
        (state, addr)
    }

    /// 本机挂着系统 HTTP 代理时，reqwest 连 127.0.0.1 也会绕过去（连不上变成代理回的 502）——
    /// 测试里一律直连，结果才不随机器环境变。
    fn no_proxy_state(cfg: Config) -> ProxyState {
        let mut st = ProxyState::new(cfg).unwrap();
        st.client = Client::builder().no_proxy().build().unwrap();
        st
    }

    fn direct() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    /// 流传完之后日志才回填，等它一下。
    async fn settled_log(state: &ProxyState) -> LogEntry {
        for _ in 0..100 {
            if let Some(e) = state.logs.read().unwrap().last().filter(|e| e.duration_ms.is_some()) {
                return e.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("日志一直没有回填");
    }

    const SSE_BODY: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1000,\"output_tokens\":1}}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":500}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    #[tokio::test]
    async fn streamed_response_is_untouched_and_the_log_gets_usage_and_cost() {
        let upstream = Router::new().fallback(|| async { ([("content-type", "text/event-stream")], SSE_BODY) });
        let pricing = ModelPricing { input: Some(2.0), output: Some(8.0), ..Default::default() };
        let (state, addr) = proxy_with_upstream(upstream, Some(pricing)).await;

        let body = direct()
            .post(format!("{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(serde_json::json!({"model": "claude-opus-5", "stream": true, "max_tokens": 5,
                                      "messages": [{"role": "user", "content": "hi"}]}).to_string())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, SSE_BODY, "旁路扫描不许改响应的任何一个字节");

        let e = settled_log(&state).await;
        assert_eq!((e.slot.as_str(), e.model.as_str(), e.status), ("claude-opus-5", "real-model", 200));
        assert_eq!(e.usage, Some(Usage { input_tokens: 1000, output_tokens: 500, ..Default::default() }));
        assert!((e.cost_usd.unwrap() - 0.006).abs() < 1e-12, "1000×2 + 500×8 每百万 token");
        let today = state.today_stats();
        assert_eq!((today.requests, today.with_usage, today.priced), (1, 1, 1));
    }

    #[tokio::test]
    async fn without_a_price_the_log_has_tokens_but_no_cost() {
        let upstream = Router::new().fallback(|| async {
            ([("content-type", "application/json")], r#"{"type":"message","usage":{"input_tokens":3,"output_tokens":4}}"#)
        });
        let (state, addr) = proxy_with_upstream(upstream, None).await;
        direct()
            .post(format!("{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(serde_json::json!({"model": "claude-opus-5", "max_tokens": 5, "messages": []}).to_string())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let e = settled_log(&state).await;
        assert_eq!(e.usage, Some(Usage { input_tokens: 3, output_tokens: 4, ..Default::default() }));
        assert_eq!(e.cost_usd, None, "没费率就不算 —— 绝不估算");
        assert_eq!(state.today_stats().priced, 0);
    }

    /// Auto 模式的安全检查在日志里标出来（抓包所见的形态：系统提示词是 block 数组，第一块是计费头）。
    #[test]
    fn auto_mode_checks_are_recognised() {
        let check = serde_json::json!({
            "model": "claude-sonnet-5", "max_tokens": 64,
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.270; cc_entrypoint=sdk-cli;"},
                {"type": "text", "text": "You are a security monitor for autonomous AI coding agents.\n\n## Context"}
            ],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "<transcript> "}]}]
        });
        assert!(is_auto_mode_check(&check));
        let as_string = serde_json::json!({"system": "You are a security monitor for autonomous AI coding agents."});
        assert!(is_auto_mode_check(&as_string));
        // 普通对话、标题生成都不算
        let chat = serde_json::json!({"system": [{"type": "text", "text": "You are a Claude agent, built on Anthropic's Claude Agent SDK."}]});
        assert!(!is_auto_mode_check(&chat));
        assert!(!is_auto_mode_check(&serde_json::json!({"messages": []})));
    }

    #[tokio::test]
    async fn an_auto_mode_check_is_tagged_in_the_log() {
        let upstream = Router::new().fallback(|| async {
            ([("content-type", "application/json")], r#"{"type":"message","usage":{"input_tokens":3,"output_tokens":1}}"#)
        });
        let (state, addr) = proxy_with_upstream(upstream, None).await;
        direct()
            .post(format!("{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(serde_json::json!({"model": "claude-opus-5", "max_tokens": 64,
                "system": [{"type": "text", "text": "You are a security monitor for autonomous AI coding agents."}],
                "messages": [{"role": "user", "content": "<transcript>"}]}).to_string())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let e = settled_log(&state).await;
        assert_eq!((e.status, e.note.as_str()), (200, NOTE_AUTO_MODE_CHECK));
    }

    #[tokio::test]
    async fn unreachable_upstream_is_logged_instead_of_vanishing() {
        // 以前这条 502 不进日志：Claude 里报错，ModelLink 的日志页却一片空白
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_url = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        let cfg = Config {
            providers: vec![Provider {
                target_url: dead_url,
                api_key: "k".into(),
                models: vec![ModelEntry { name: "real-model".into(), ..Default::default() }],
                thinking_effort: String::new(),
            }],
            ..Default::default()
        };
        let state = Arc::new(no_proxy_state(cfg));
        let pl = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("http://{}", pl.local_addr().unwrap());
        tokio::spawn(serve(pl, state.clone()));

        let resp = direct()
            .post(format!("{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(serde_json::json!({"model": "claude-opus-5", "max_tokens": 5, "messages": []}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 502);
        assert!(resp.text().await.unwrap().starts_with("Proxy error: "), "响应话术不变");
        let e = settled_log(&state).await;
        assert_eq!((e.status, e.error, e.note.as_str()), (502, true, "连不上服务商"));
        assert!(!e.detail.is_empty());
        assert_eq!(state.today_stats().failures, 1);
    }

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
