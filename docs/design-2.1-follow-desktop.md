# ModelLink 2.1 设计方案：跟随 Claude Desktop 进化

> 基线：Claude Desktop **1.46388.3**（2026-09-04 构建）解包所得 schema，143 个配置键。
> ModelLink 当前实现基线：最后提交 `042acf4`（2026-07-14）。
> 本文所有机制结论均来自 app.asar 实测，不是文档推测；标注「待验证」的除外。

---

## 一、三个决定性机制（本次设计的地基）

### 1.1 推理强度选择器完全由 slot 名决定 ⭐ 最重要

app 内有一张硬编码表 `Vwt`，按**模型 ID 精确匹配**决定模型选择器里出不出现 effort / mode 选项：

```js
Vwt = {
  "claude-haiku-4-5":  { modes:["extended"] },
  "claude-sonnet-4-5": { modes:["extended"] },
  "claude-sonnet-4-6": { effortLevels:["low","medium","high","max"],          recommended:"low",    modes:["auto"] },
  "claude-sonnet-5":   { effortLevels:["low","medium","high","xhigh","max"],  recommended:"medium", modes:["auto"] },
  "claude-opus-4-6":   { effortLevels:["low","medium","high","max"],          recommended:"medium", modes:["extended"] },
  "claude-opus-4-7":   { effortLevels:["low","medium","high","xhigh","max"],  recommended:"xhigh",  modes:["auto"] },
  "claude-opus-4-8":   { effortLevels:["low","medium","high","xhigh","max"],  recommended:"high",   modes:["auto"] },
  "claude-opus-5":     { effortLevels:["low","medium","high","xhigh","max"],  recommended:"high",   modes:["auto"] },
}
Hwt = /^(?:claude-)?(?:fable|mythos)(?:-|$)/   // fable / mythos 走 Bwt 默认档
```

**ModelLink 现在用的 8 个 legacy slot 一个都不在表里** ——
`claude-3-opus-latest` / `claude-3-5-sonnet-latest` / `claude-3-sonnet-20240229` / `claude-3-haiku-20240307` /
`claude-3-5-haiku-latest` / `claude-3-opus-20240229` / `claude-3-5-sonnet-20241022` / `claude-3-5-sonnet-20240620`

所以 Claude Desktop 里**没有推理强度选择器**，这正是 `proxy.rs::inject_thinking()` 存在的原因：ModelLink 只能在代理里按「服务商级」硬注入 `output_config.effort`，用户没法逐次选。

换成 `Vwt` 表里的 slot 之后，Claude Desktop 会**原生渲染 5 档 effort 选择器 + auto 模式**，app 自己把 `output_config.effort` 带进请求体。代理的角色从「硬注入」变成「按上游能力翻译」。

这条正好接上上游现实：Kimi `k3` 的 `/v1/models` 返回
`think_efforts: { valid_efforts: ["low","high","max"], default_effort: "max" }` —— 需要档位映射，不是直通。

### 1.2 系统提示词的模型身份由 slot 名派生

```js
ds = Map([["claude-3-7-sonnet","Claude 3.7 Sonnet"],["claude-3-5-sonnet","Claude 3.5 Sonnet"],["claude-3-5-haiku","Claude 3.5 Haiku"]])

fs(id):  查 ds → 否则匹配 /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?$/ 生成 "Opus 5" → 否则 undefined

ps({modelId, provider, labelOverride}):
  r = fs(modelId)
  base = r === undefined
       ? `You are powered by the model ${id}.`
       : `You are powered by the model named ${r}. The exact model ID is ${id}.`
  return labelOverride ? `${base} The administrator of this deployment has labeled this model "${labelOverride}".` : base
```

三种 slot 命名的实际效果：

| slot | 系统提示词 | effort 选择器 |
|---|---|---|
| `claude-3-opus-latest`（现状） | "You are powered by the model claude-3-opus-latest." | ❌ |
| `claude-opus-5` | "…the model named **Opus 5**. The exact model ID is claude-opus-5." | ✅ 5 档 |
| `claude-ml-1`（中性名） | "…the model named **Ml 1**." | ❌ |

**权衡**：要 effort 选择器就必须借用真实 Claude 名，代价是系统提示词声称自己是 Opus 5。

**实测结论：这个代价不存在（Kimi k3，2026-09-10）**

上游模型自己的身份训练压得过系统提示词。四组对照，全部回答"我是 Kimi"：

| 系统提示词 | 模型回答 |
|---|---|
| ① 无 | 我是 Kimi，由 Moonshot AI（月之暗面）开发 |
| ② `You are powered by the model named Opus 5. The exact model ID is claude-opus-5.` | 我是 Kimi……具体底层模型版本信息我这里不显示 |
| ③ ② + `The administrator of this deployment has labeled this model "Kimi K3".` | 我是 Kimi；当前部署把该模型标为 **Kimi K3**。系统同时显示模型名为 Opus 5，模型 ID 是 claude-opus-5 |
| ④ `You are Claude Code, Anthropic's official CLI for Claude.` + ② | 我是 Kimi，由月之暗面开发 |

两个推论：

1. **借用真实 Claude slot 名不会造成身份冒充**，§2.1 的方案可以放心用
2. **`labelOverride` 本身就是身份信息的载体** —— 对照 ② 和 ③：不带 labelOverride 时模型只知道自己是 Kimi，说不出是哪个版本；带上之后它能完整报出三层信息。所以 `labelOverride` 应该填**人类可读的准确型号**（`Kimi K3` 而非 `kimi-k3`），它同时服务于选择器显示和模型自我描述

⚠️ 仅在 Kimi k3 上验证。部分国产模型因训练数据污染会自称 Claude / GPT，**每接入一家新服务商时应跑一次这个对照**（4 行脚本，见 §六）。

### 1.3 名字过滤器与费率的交互

- 过滤器要求模型名含 `claude|opus|sonnet|haiku|fable|mythos|anthropic` 任一，且不命中约 50 项的第三方厂商拒绝正则（`kimi`/`glm`/`deepseek`/`qwen`/…）
- `inferenceModels` 官方上限 **200 条**，`MAX_MODELS = 8` 是自我设限
- 但费率注解写明："A built-in Claude ID also covers its dated and provider forms" —— **用内置 Claude ID 当 slot，引擎会按 Anthropic 官方价估算**，除非 `inferenceModelPricing` 给出覆盖行

⚠️ **强耦合**：§3.6（放开模型数 / 改 slot）与 §3.1（真实费率）必须**同批交付**，否则用户会看到按 Opus 官方价计的假账单。

---

## 二、slot 命名的重新设计

### 2.1 新 slot 池（按能力分层）

```rust
pub const SLOT_POOL: &[Slot] = &[
    // 一线：5 档 effort + auto 模式
    Slot { id: "claude-opus-5",     efforts: &["low","medium","high","xhigh","max"], recommended: "high"   },
    Slot { id: "claude-sonnet-5",   efforts: &["low","medium","high","xhigh","max"], recommended: "medium" },
    Slot { id: "claude-opus-4-8",   efforts: &["low","medium","high","xhigh","max"], recommended: "high"   },
    Slot { id: "claude-opus-4-7",   efforts: &["low","medium","high","xhigh","max"], recommended: "xhigh"  },
    // 二线：4 档
    Slot { id: "claude-opus-4-6",   efforts: &["low","medium","high","max"],         recommended: "medium" },
    Slot { id: "claude-sonnet-4-6", efforts: &["low","medium","high","max"],         recommended: "low"    },
    // 三线：仅 extended 模式，无 effort
    Slot { id: "claude-sonnet-4-5", efforts: &[],                                    recommended: ""       },
    Slot { id: "claude-haiku-4-5",  efforts: &[],                                    recommended: ""       },
    // 溢出层：无 effort，纯占位，可无限扩
    // "claude-ml-1" … "claude-ml-N"
];
```

**分配策略**：按用户在链路板上的顺序，优先给一线 slot；超过 8 个后用 `claude-ml-{n}` 溢出层。
上限从 8 提到 **20**（留足余量，远低于 200 的硬上限）。

### 2.2 老用户迁移

`config.json` 里存的是 providers + models，**不存 slot**（slot 在 `flatten_config` 时按顺序现算）。
所以改 slot 池**不需要迁移配置文件** —— ~~但会改变 `config_hash`~~
**⚠️ 已证伪：不会改变**（`canonical_hash` 只覆盖 providers + port，槽位是 flatten 时现算的）。
实现里补了 `SLOT_POOL_VERSION` 进哈希才达成本节的意图，见 §七②。提示语：

> 新版为你的模型启用了 Claude Desktop 原生推理强度选择器，点「应用到 Claude Desktop」生效。

---

## 三、八项改动的详细设计

### 3.1 真实费率表（⭐⭐⭐ 独占卖点）

**写入键**

```json
"inferenceModelPricingEnabled": true,
"inferenceModelPricing": [
  { "name": "claude-opus-5",
    "inputPerMtok": 0.58, "outputPerMtok": 2.32,
    "cacheReadPerMtok": 0.12, "cacheWritePerMtok": 0.72 }
]
```

- 单位固定 **USD / 百万 token**
- ~~四个字段全部可选~~ → **⚠️ 已证伪，四个字段全部必填**（2026-09-10 实测，见 §七①）
- `inferenceModelPricingMultiplier` 取值域 **(0, 1]** —— 只能打折，**不能当汇率用**
- 每个 slot 一行，`name` = slot ID（不是上游真名）

**改动**
- `config.rs`：`ProviderModel` 增 `pricing: Option<ModelPricing>`（4 个 f64 + 币种）
- `gateway.rs::inference_models_entries` 旁边新增 `inference_model_pricing_entries()`
- ~~预设库：为 8 家内置服务商填官方价（人民币），换算成美元写入~~
  → **改为运行时从 models.dev 同步**（2026-09-10 定，见 §七③）
- UI：模型编辑器加「费率」区，同步值作占位显示、手填可覆盖

**待决策 ①** 汇率怎么处理，见 §五。

### 3.2 SSE keep-alive 注入 + `inferenceStreamIdleTimeoutSec`（⭐⭐⭐ 治断流）

`inferenceStreamIdleTimeoutSec`（1.44121.1，取值 300–1800，默认 300）**只在网关往响应里写 SSE keep-alive 时才生效** ——
原文：*"It only helps when the gateway writes SSE keep-alive `ping` events (or `:` comment lines) into the response while the upstream model is silent."*
而且：*"A response on which nothing at all arrives — no pings — still fails after about 5 minutes regardless of this key."*

ModelLink 的代理正站在中间，这是**只有本地代理型工具能做**的能力。

**改动（`proxy.rs`）**

当前是 `Body::from_stream(resp.bytes_stream())` 直通。改成带心跳的合流：

```
上游 chunk  ──┐
              ├─► select! ─► 下游
心跳 tick ────┘   （仅在距上次上游 chunk ≥ 15s 时才发 `: ping\n\n`）
```

要点：
- 只对 `stream: true` 的请求启用；非流式不动
- 心跳用 SSE 注释行 `: ping\n\n` —— 合法且不会被解析成事件，比伪造 `event: ping` 安全
- 间隔 15s，可配置
- 上游一有数据就重置计时器

**同时写入**：`"inferenceStreamIdleTimeoutSec": 1800`

### 3.3 干掉静默回落（⭐⭐⭐）

现状 `config.rs::resolve_model()` 末尾：

```rust
if let Some(e) = flat.into_iter().next() {
    eprintln!("  fallback: {} -> {}", model, resolved);   // 只打日志，照发不误
```

**双层静默**：ModelLink 回落到第一个模型；而上游也会静默回落 ——
实测 Kimi `/coding/` 端点对 `banana`、空字符串、`claude-opus-5` 全部返回 200，直接给默认模型。
用户完全不知道自己在用什么。

**改动**
- `resolve_model` 返回 `Result<ResolvedModel, ResolveError>`，无匹配返回 `ResolveError::UnmappedSlot(slot)`
- `proxy.rs` 返回 `400` + Anthropic 错误体：
  ```json
  {"type":"error","error":{"type":"invalid_request_error",
    "message":"ModelLink: 模型槽位 claude-opus-5 未映射到任何服务商。请在 ModelLink 中配置后重试。"}}
  ```
- 请求日志页把这类请求标红
- 保留一个「兼容模式」开关（默认关），给依赖旧行为的用户台阶

### 3.4 补两个一直没写的键（⭐⭐）

| 键 | 值 | 效果 |
|---|---|---|
| `chatTabEnabled` | `true` | Chat 标签页默认是**关**的，1.13576.0 起需显式开 |
| `disableDeploymentModeChooser` | `true` | 免去每次启动选部署模式 |

各一行，`gateway.rs::apply_to_claude` 里加。

### 3.5 模型条目补三个字段（⭐⭐）

当前只写 `name` / `supports1m` / `labelOverride`。补：

```json
{ "name": "claude-opus-5", "labelOverride": "Kimi K3",
  "supports1m": true, "prefer1m": true,
  "anthropicFamilyTier": "opus", "isFamilyDefault": true }
```

- `prefer1m`：该条为默认模型时，1M 变体成为选择器默认项（无 `supports1m` 时无效）
  ⚠️ **前提是这个模型真有 1M** —— 预设里多数没有，见 §七⑤
- `anthropicFamilyTier`（`sonnet`/`opus`/`haiku`/`fable`/`mythos`）：把裸别名 `opus` pin 到这条；opus/fable 还带 refusal fallback 链路
- `isFamilyDefault`：同 tier 多条时指定哪条接管别名

UI：模型编辑器加「层级」下拉（可留空）。

### 3.6 放开 8 个上限 + 换 slot 池（⭐⭐）

- `MAX_MODELS: 8 → 20`
- `ANTHROPIC_SLOTS` → `SLOT_POOL`（§2.1），溢出走 `claude-ml-{n}`
- `flatten_config` 分配逻辑相应调整

**必须与 §3.1 同批发布**（否则假账单）。

### 3.7 `organizationInstructions`（⭐ 降级：从「身份纠正」改为「用户自定义指令」）

> **本节已按实测修订。** 原设计假设需要用它纠正模型身份，§1.2 的对照实验证否：
> 上游模型的身份训练压得过系统提示词，`labelOverride` 已经把型号信息传达到位。

保留这个键，但用途改成**纯用户自定义**：

- 设置页一个多行输入框，上限 **3000 字符**（app 硬限制）
- 内容原样写入 `organizationInstructions`，ModelLink 不拼接任何自动生成文本
- 说明文案：追加到 Chat / Cowork / Code 的系统提示词（含它们派生的子 agent），
  app 会告诉模型「这来自管理员，优先于用户个人偏好」；是引导不是强制约束
- 典型用法：统一语言风格、代码规范、禁谈话题

**兜底开关（2026-09-10 实测定稿：默认开，且确认有效，见 §七④）**：
勾选项「在指令前追加真实模型说明」，勾上才拼接：

```
你的真实模型是 {labelOverride}，由 ModelLink 本地网关路由。
系统提示词中出现的 Claude 模型名只是路由槽位。
```

接入新服务商时用 §六的对照脚本判断要不要勾。

### 3.8 版本自适应写入（⭐）

- 读 Claude Desktop 版本：macOS `Claude.app/Contents/Info.plist` 的 `CFBundleShortVersionString`；Windows 从安装目录读
- 内置 `键 → availableInVersion` 表，低于门槛的键不写
- 已知门槛：`chatTabEnabled` 1.13576.0 / `inferenceModelPricing*` 1.37937.0 / `inferenceStreamIdleTimeoutSec` 1.44121.1 / `egressProxy*` 1.44121.1
- 设置页显示检测到的版本 + 哪些能力因版本过低不可用

**顺带盯一个到期点**：`inferenceGatewayAuthScheme` 的 `sso` / `auto` 值 **2026-10-07 失效**。
ModelLink 写的是 `bearer`，安全，但值得在代码里加注释免得以后踩。

### 3.10 修 `inject_thinking` 覆盖桌面端 effort 的 bug（⭐⭐⭐ 与 §2.1 同批必修）

**现状**（`proxy.rs`）：

```rust
pub(crate) fn inject_thinking(data: &mut Value, te: &str) -> String {
    if !te.is_empty() && te != "off" {
        data["thinking"]      = json!({"type": "enabled", "budget_tokens": 8192});
        data["output_config"] = json!({"effort": te});   // ← 直接赋值，桌面端传的被丢弃
    } else if te == "off" {
        data["thinking"] = json!({"type": "disabled"});
        data.as_object_mut().map(|o| o.remove("output_config"));
    }
    …
}
```

这段逻辑写于「桌面端没有 effort 选择器」的年代，是当时唯一能设推理强度的办法。
换用 §2.1 的新 slot 池后**桌面端会原生渲染 5 档选择器并把值发过来**，
而现有代码会无条件覆盖，等于用户在桌面端选了个寂寞。

**改成三态语义（服务商级设置降级为「默认档位」）**

| 服务商级 `thinking_effort` | 请求里有 `output_config.effort` | 行为 |
|---|---|---|
| 任意 | ✅ 有 | **原样透传**，服务商级设置不参与 |
| `""`（未设置） | ❌ 无 | 不注入，交给上游默认 |
| `off` | ❌ 无 | `thinking: {type:"disabled"}` + 移除 `output_config` |
| 其它档位 | ❌ 无 | 注入该档位作为兜底默认 |

要点：

- `budget_tokens: 8192` 这个硬编码值也一并去掉 —— 与 `output_config.effort` 语义重叠，
  且对 1M 上下文模型明显偏小。~~仅在 `thinking_effort` 兜底注入时保留~~
  → **兜底注入里也不留了**，改发桌面端同款的 `thinking:{"type":"adaptive"}`，见 §七⑩
- UI 文案要改：服务商编辑器里「推理强度」改成「**默认推理强度**」，
  副标题注明「桌面端选择器优先；此处仅在桌面端未指定时生效」
- 老用户迁移：已设过服务商级 effort 的用户，行为从「强制」变「默认」。
  这是修 bug 不是破坏性变更，但发版说明要写清楚

### 3.11 错误整流层：上游报错自动修复重试（⭐⭐⭐ 补短板）

**问题**：第三方 Anthropic 兼容端点的兼容度参差不齐，同一个请求体在官方能过、在某家就 400。
用户看到的是一句原始英文错误，无从下手。这是当前 ModelLink 最大的体验短板 —— 代理是纯转发，
上游报错就原样吐给桌面端。

**方案**：在 `proxy.rs` 的转发层加一道「错误驱动的整流 → 重试」。
上游返回 4xx 时匹配已知错误形态，改写请求体后重试一次；成功则用户完全无感，失败则原样返回。

```
上游响应 4xx
   ↓ 匹配错误形态
改写请求体（每类一个 rectifier）
   ↓
重试一次 ——→ 成功：正常返回，请求日志标注「已自动修复：<类型>」
   ↓ 仍失败
原样返回原始错误 + 附上「尝试过的修复」说明
```

**通用约束**

- 每个请求最多整流 **2 次**，同类整流只做一次 —— 防循环
- 每类 rectifier 独立开关，总开关默认**开**（这是修复不是优化，默认该开）
- 整流只在**非流式或流尚未开始**时可做；已经吐出 SSE 事件的请求不重试
- 每次整流写请求日志，链路板上的模型 chip 显示修复标记

#### 3.11.1 effort 不被接受（⭐ 最高优先，ModelLink 独有价值）

引擎自己也有这条逻辑，但它的处理方式是**锁定整个会话不再发 effort**
（§5.5.2 抓包所见的 `latching unsupported`）—— 用户此后在选择器里怎么调都没用，且毫无提示。

在代理层拦下来更好：**桌面端根本不会看到这个错误，也就不会锁定会话**。

| 触发 | 上游 4xx 且错误消息含 `output_config` 或 `effort`，或去掉 `output_config` 后重试即成功 |
|---|---|
| 动作 | 移除 `output_config`，按 §3.11.4 的能力表换用该服务商支持的思考参数，重试 |
| 副作用 | 在服务商能力缓存里标记 `effort_supported = false`，后续请求直接不发，省掉每次的失败往返 |
| 提示 | 服务商编辑器上打一个「该服务商不支持推理强度调节」的标记 |

#### 3.11.2 thinking budget 约束

| 触发 | 错误消息**同时**含 `budget_tokens`（或 `budget tokens`）+ `thinking` + 1024 下限约束（`greater than or equal to 1024` / `>= 1024` / `1024` 且 `input should be`） |
|---|---|
| 动作 | `thinking.type = "enabled"`；`thinking.budget_tokens = 32000`；若 `max_tokens < 32001` 则提到 `64000` |
| 例外 | `thinking.type == "adaptive"` 的请求**不改写**（自适应思考没有 budget 概念，改了反而报新错） |

#### 3.11.3 thinking 块结构 / 签名

多轮对话里，assistant 历史消息带的 `thinking` / `redacted_thinking` block 和 `signature` 字段
是 Anthropic 官方的加密产物，第三方端点大多验不了或直接拒收。

| 触发（任一） | 动作 |
|---|---|
| `invalid` + `signature` + `thinking` + `block` | 移除 messages 里全部 `thinking` / `redacted_thinking` block |
| `thought signature` + (`not valid` / `invalid`) | 及其 `signature` 字段，重试 |
| `must start with a thinking block` | 同上 |
| `expected` + (`thinking` / `redacted_thinking`) + `found` + `tool_use` | 同上 |
| `signature` + `field required` | 同上 |
| `signature` + `extra inputs are not permitted` | 同上 |

整流结果要统计：移除了几个 thinking block、几个 redacted block、几个 signature 字段，写进日志。

#### 3.11.4 服务商思考能力表（整流的知识底座）

`ProviderPreset` 增 `thinking_caps`，由 §5.6 的探针填充，整流命中时自动更新：

```rust
pub struct ThinkingCaps {
    effort_supported:        Option<bool>,   // output_config.effort
    adaptive_supported:      Option<bool>,   // thinking:{type:"adaptive"}
    enabled_budget_supported:Option<bool>,   // thinking:{type:"enabled",budget_tokens}
    can_disable:             Option<bool>,   // thinking:{type:"disabled"}
    accepts_thinking_blocks: Option<bool>,   // 历史消息里的 thinking block
}
```

`None` = 未知，先按「支持」乐观发送，被拒后由整流层落为 `false` 并持久化。

#### 3.11.5 槽位名与真实能力的解耦（⚠️ 必须避开的坑）

§2.1 之后 slot 名是 `claude-opus-5` 这类**借用来的 Claude 型号名**，
和背后真实上游模型（可能是 GLM、Kimi、DeepSeek）**毫无关系**。

所以整流层和任何思考参数决策，**一律不得从 slot 名推断能力** ——
必须查 `ThinkingCaps`（按服务商 + 真实上游模型 ID 索引）。

作为对照，Claude Desktop 自己的 `Vwt` 表按 slot 名决定**选择器 UI**，那是对的（UI 归 UI）；
但**发给上游的参数**必须走能力表。两者不能混。

供参考，Desktop 自带的 slot → UI 能力对应关系（从 app.asar 提取）：

| slot | effort 档位 | 思考模式 |
|---|---|---|
| `claude-opus-5` / `claude-opus-4-8` / `claude-opus-4-7` / `claude-sonnet-5` | low / medium / high / xhigh / max | auto |
| `claude-opus-4-6` | low / medium / high / max | extended |
| `claude-sonnet-4-6` | low / medium / high / max | auto |
| `claude-sonnet-4-5` / `claude-haiku-4-5` | 无 | extended |
| `fable*` / `mythos*`（正则 `^(?:claude-)?(?:fable\|mythos)(?:-\|$)`） | 走默认档 | — |

### 3.9 附加：网络代理透传（原第 5 项，降级为附带）

`egressProxyUrl` / `egressProxyPacUrl`（1.44121.1，`[3p,1p]` 双作用域）。
给挂梯子或公司代理的用户。约束：

- 只接受 `http://` / `https://`，**SOCKS 被拒**，**不接受内嵌账号密码** `user:pass@`
- `localhost` / `127.0.0.1` / `[::1]` / `*.local` 自动 bypass（ModelLink 的 127.0.0.1 网关不受影响）
- 代理不通**直接失败，不回落直连**
- 只能从 MDM 或本地配置文件读，**启动时读一次，改了要重启 app**

UI：设置页加一个可选输入框，带上述约束的校验提示。

---

## 四、分批交付

| 批次 | 内容 | 可独立验证 |
|---|---|---|
| **A** | §3.3 静默回落 + §3.4 两个键 | 单测：`resolve_model` 返回 Err；手测：Chat 页出现 |
| **B** | §2.1 slot 池 + §3.6 放开上限 + §3.1 费率表 | 手测：选择器出现 effort 5 档；Usage 页显示真实费率 |
| **C** | §3.2 SSE 心跳 + `inferenceStreamIdleTimeoutSec` | 造一个 60s 沉默的上游 mock，验证不断流 |
| **C+** | **§3.11 错误整流层** | mock 上游返回各类错误，验证整流后重试成功；防循环上限 |
| **D** | §3.5 模型条目字段 + §3.7 organizationInstructions | 手测：问模型"你是谁" |
| **E** | §3.8 版本自适应 + §3.9 网络代理 + §5.6 探针进「测试连接」 | 降级到旧版 Desktop 验证不写新键 |

A 批改动最小、风险最低，建议先发一个 2.0.x 补丁；B/C 合为 2.1.0。

§3.11 中的 **3.11.1（effort 整流）** 可以提前到 A 批 —— 它和 §3.10 改的是同一个函数，
一起做能省一次回归；而且它把「用户调了 effort 没反应」这个隐性故障变成可见、可自愈，
是 A 批里用户感知最强的一项。

---

## 五、已定的决策

### ① 费率货币 → 设置页填汇率，默认 7.2

`inferenceModelPricing` 单位写死 USD/百万 token，`multiplier` 取值域 (0,1] 当不了汇率（>1 被拒）。

**方案**：预设库存**人民币原价**，设置页一个「人民币兑美元汇率」输入框（默认 `7.2`），
写入 `inferenceModelPricing` 时统一除以汇率换算成 USD。

- 汇率是全局的，不是逐服务商
- 预设库里的价格保持人民币，便于对照各家官网价目表校验
- 已经用美元计价的服务商（少数海外中转），模型条目上加 `currency: "USD"` 标记，跳过换算

理由：内置固定汇率会过时且不可见，直接填人民币又会显示成 `$` 误导用户。ModelLink 的定位是「把账算清楚」，汇率必须可见可改。

### ② ~~`organizationInstructions` 要不要开放自定义~~ —— 已由实测证否

身份纠正的前提被 §1.2 的对照实验推翻，此项无需决策：降级为纯用户自定义输入框 + 一个默认关闭的兜底开关。

### ③ effort → **默认原样透传**，映射表降级为可选覆盖

> **本节已按 Kimi 官方文档修订。** 原设计假设需要 ModelLink 做档位映射，事实是
> **服务商自己在网关侧就映射好了**，而且映射方向和直觉相反。

**线上格式（引擎二进制实测）**

- effort 走请求体 `output_config.effort`，配 `anthropic-beta: effort-2025-11-24` 头
- 桌面端 5 档 `low / medium / high / xhigh / max`（`xhigh` 在 UI 上显示为 **Extra**）
- 引擎有**自动降级**：上游若拒绝该字段，引擎打 `[effort] model X rejected output_config.effort;
  latching unsupported and retrying without it`，去掉 effort 重试，**并锁定整个会话不再发**
  （telemetry `tengu_effort_unsupported_retry`）。所以上游不支持时不会炸，但会静默失效
- 另有 clamp 逻辑：thinking 被禁用时，部分模型会拒绝高档位，引擎自动下钳

**Kimi 官方映射表**（K3 支持 `low/high/max` 三档，网关侧自动换算）

| Claude Code 档位 | K3 实际档位 |
|---|---|
| `low` | `low` |
| `medium` | `high`（推荐） |
| `high` | `high`（推荐） |
| `xhigh` | `max` |
| `max` | `max` |
| 未设置 | `high` |

注意 `medium → high` —— 服务商按「质量优先」映射，不是线性降级。ModelLink 自作主张映射会**降低质量**。

**方案**

1. **默认透传**：`ProviderPreset.effort_map` 默认为空 = 不改写 `output_config.effort`
2. 只有明确已知会拒绝该字段的服务商才填映射表，用户可在服务商编辑器覆盖
3. 「测试连接」时读 `/v1/models` 的 `think_efforts.valid_efforts`（Kimi 会返回），
   **只在 UI 上提示**上游支持哪几档，不自动改写映射
4. 真发生映射/丢弃时记进请求日志，与 §3.3 的透明原则一致

**必须一起修的 bug（§3.10）**：`proxy.rs::inject_thinking()` 目前无条件覆盖 `output_config`，
会把桌面端传来的 effort 吃掉。见下节。

---

## 五点五、抓包实测：桌面端到底往网关发什么

> 工具：`~/kimi-k3-proxy/capture.py`（转发式抓包代理，记录请求/响应结构到 JSONL）。
> 环境：Claude Desktop 1.46388.3 + Chat 界面 + slot `claude-opus-5` / labelOverride `kimi-k3` → Kimi k3。

### 5.5.1 Chat 模式跑的就是 Claude Code 引擎

```
path      : POST /v1/messages?beta=true
betas     : claude-code-20250219, context-1m-2025-08-07, interleaved-thinking-2025-05-14
system    : 20,243 字符
tools     : 28 个 —— AskUserQuestion / Edit / Read / Skill / WebSearch / Write
                    + mcp__cowork__{create_artifact,list_artifacts,present_files,…}
max_tokens: 64000
顶层多一个 metadata 字段
```

系统提示词的头三块：

```
[0] "x-anthropic-billing-header: cc_version=2.1.260.222; cc_entrypoint=local-agent;"
[1] "You are a Claude agent, built on Anthropic's Claude Agent SDK."   ← cache_control ttl=1h
[2] "<application_details>\nClaude is powering Chat mode, a feature of the Claude desktop app…"
```

**这推翻了 §1.2 的乐观结论**：第 [1] 块是**第二人称角色断言**，比 `ps()` 生成的第三人称能力描述强硬得多。
Kimi 在 Chat 里会回答「我是 Claude，由 Anthropic 开发」，思考过程原样复述了这两句。
→ §3.7 的兜底开关**默认打开**。
**2026-09-10 已实测定稿**：开了之后 Kimi 在 Chat 里答「我是 Kimi-k2.6，由月之暗面开发。
这里的 `claude-opus-5` 只是网关路由槽位名，不代表我的真实身份」—— `organizationInstructions`
压得住第 [1] 块的角色断言。见 §七④。

### 5.5.2 effort 原样透传，无改写无钳制 ✅

```
output_config={'effort': 'low'}   thinking={'type': 'adaptive'}
output_config={'effort': 'high'}  thinking={'type': 'adaptive'}
output_config={'effort': 'max'}   thinking={'type': 'adaptive'}
```

值与用户在选择器里选的完全一致，未触发「上游拒绝 → 去 effort 重试」。

⚠️ 桌面端**同时发 `thinking: {"type":"adaptive"}`**。
现有 `inject_thinking` 写死的 `{"type":"enabled","budget_tokens":8192}` 会把自适应思考踩掉 —— §3.10 一并处理。

### 5.5.3 effort 对 Kimi k3 真实生效（受控对照）

同一问题、同一上下文、仅 effort 不同，各 2 次采样（直连 Kimi，不经代理）：

| effort | thinking tokens | output tokens | 耗时 |
|---|---|---|---|
| `low`  | 2326 / 2215（均 2271） | 2635 / 2737 | ~57s |
| `high` | 2998 / 3060（均 3029） | 3629 / 3694 | ~76s |

low → high 思考量 **+33%**，耗时 **+33%**，组内方差 2–5%。

> 早前在同一会话里连测三档得到 `low=5 / high=5 / max=32`，是因为对话过短、思考需求触底，
> 且消息数累积（2 → 5 → 8）造成上下文不可比。**测 effort 必须开独立会话或离线重放。**

### 5.5.4 两个隐藏调用 —— 可优化的成本点 💰

| 请求特征 | 用途 | 消耗 |
|---|---|---|
| `max_tokens=1`，正文 `"."`，无 system，无 tools | **连接健康检查** | 8 in / 1 out |
| `max_tokens=200`，user 以 `"You are coming up with a succinct title for an agen…"` 开头 | **会话标题生成** | 451 in / 110 out，**其中 88 thinking** |

标题生成**不带 `output_config`**，上游按自己的默认档跑（Kimi 默认 `high`），
花 88 个思考 token 只为起个标题。对 `supports_thinking_type: "only"` 的模型是纯浪费，且每开一个新会话来一次。

**优化点（竞品无人做过，且好做 demo）**

- 标题生成：识别特征后注入 `output_config: {effort: "low"}` 或 `thinking: {type: "disabled"}`
- 健康检查：本地短路直接返回，不打上游
- 两者都要能在设置里关掉，并在请求日志里标注「已优化」

### 5.5.5 顺带确认

- **Prompt caching 生效**：`cache_read_input_tokens: 9728` —— Kimi 完整透传 `cache_control` ✅
- **`GET /v1/models?limit=1000` 确实被调用** —— 说明 Model discovery 开关被显式打开时不会跳过。
  Kimi 返回的 4 个 ID 全被名字过滤器删光，纯浪费一次往返 → ModelLink 应写 `modelDiscoveryEnabled: false`
- 请求不带 `temperature`

---

## 五点六、待办：其余七家服务商的能力探测

**状态：挂起**，等有 key 或查到官方文档再补。

工具已就绪：`gui-tauri/scripts/probe-provider.py`

```bash
python3 probe-provider.py <base_url> <api_key> <model_id>
```

一次探测 7 项，输出可直接填进 `presets.ts`：

1. 基础 `POST /v1/messages`
2. `GET /v1/models` + `anthropic_family_tier` + `think_efforts.valid_efforts`
3. **模型名校验严格度**（发 `zzz-nonexistent` 看 200 还是 400 —— 判断会不会像 Kimi 那样静默回落）
4. `output_config.effort` 五档接受度
5. 原生 `thinking` 三形态（`adaptive` / `enabled+budget_tokens` / `disabled`）
6. Prompt caching 透传 —— ⚠️ 该项**会假阴性**，见 §七⑥
7. `context-1m-2025-08-07` beta 头接受度

待补表（Kimi 一列已由实测填出）：

| 服务商 | effort 五档 | 原生 thinking | 名字校验 | cache 透传 | 1M beta |
|---|---|---|---|---|---|
| Kimi Code | ✅ 全接受，网关侧自映射 | 未测 | ❌ **静默回落** | ✅ | ✅ |
| DeepSeek | ? | ? | ? | ? | ? |
| GLM（智谱） | ? | ? | ? | ? | ? |
| MiniMax | ? | ? | ? | ? | ? |
| 百炼 Coding / Token | ? | ? | ? | ? | ? |
| mimo | ? | ? | ? | ? | ? |

**这套探测逻辑本身应该变成「测试连接」按钮的内容** —— 现状只判断通不通，
升级后能直接告诉用户「这家支持推理强度调节 / 这家会静默回落模型 / 这家不透传缓存」。
列为 §3.11，排在 E 批。

---

## 六、本次调研中顺带确认的其它事实（备查）

- Claude Desktop 数据目录已是 `Claude-3p`（macOS `~/Library/Application Support/Claude-3p`），ModelLink 已正确适配
- `configLibrary/` + `_meta.json.appliedId` 机制，ModelLink 固定 UUID `a0a0a0a0-b1b1-4c2c-9d3d-e4e4e4e4e4e4`，运行正常
- 模型发现：`inferenceModels` 全是完整 ID 时 app 自动跳过 `GET /v1/models`，ModelLink 的 `/v1/models` 实际很少被调用
- `anthropic_family_tier` 无法绕过名字过滤器 —— `YBt` 末行 `a.filter(t => Ha(e, t.id).ok)` 对发现结果**二次过滤**
- 网关必须实现 `POST /v1/messages`（streaming + tool use）；`GET /v1/models` 可选
- 强烈建议透传 `cache_control`，否则每轮全量重算。ModelLink 是透明转发，天然满足
- `[1m]` 后缀由引擎剥离，改发 `anthropic-beta: context-1m-2025-08-07` 请求头 —— 上游收到的是裸 ID

### 接入新服务商时的身份对照脚本

判断该服务商的模型会不会被系统提示词带偏、需不需要打开 §3.7 的兜底开关：

```python
import json, urllib.request
BASE, KEY, MODEL = "https://…", "sk-…", "…"          # 换成目标服务商
def ask(label, system):
    p = {"model": MODEL, "max_tokens": 1600,
         "messages": [{"role": "user", "content": "你是什么模型？"}]}
    if system: p["system"] = system
    h = {"Authorization": "Bearer " + KEY, "anthropic-version": "2023-06-01",
         "content-type": "application/json"}
    r = urllib.request.urlopen(urllib.request.Request(
        BASE + "/v1/messages", data=json.dumps(p).encode(), headers=h), timeout=150)
    d = json.loads(r.read().decode())
    print(label, "->", "".join(c.get("text","") for c in d["content"]
                               if c.get("type") == "text").strip()[:200])

ask("基线", None)
ask("身份句", "You are powered by the model named Opus 5. The exact model ID is claude-opus-5.")
ask("身份句+label", 'You are powered by the model named Opus 5. The exact model ID is '
                   'claude-opus-5. The administrator of this deployment has labeled this model "X".')
```

**判据**：后两组仍如实报出真实厂商 → 兜底开关保持关闭；出现自称 Claude / Opus → 打开。

---

## 七、实测修订（2026-09-10 实施期间）

A–E 全部批次已实现。以下是**实施过程中被实测推翻或需要补充的地方**，均以 app.asar
的 schema 或真实上游响应为准，原文保留在上方并就地标了指向本节的记号。

### ① §3.1「四个价格字段全部可选」→ 证伪，四个全必填

app.asar 里四个价格字段共用同一个构造器：

```js
BCe = () => O().min(0).max(1e4)     // 注意：没有 .optional()
inputPerMtok: ho(BCe(), {...}), outputPerMtok: ho(BCe(), {...}),
cacheReadPerMtok: ho(BCe(), {...}), cacheWritePerMtok: ho(BCe(), {...})
```

对照同文件里 `discoveryEnabled: k().optional()`，差别很明确。字段说明也写着
"all four required"。缺字段的行会被 schema 拒掉，而**一行不合法就可能让整张费率表
（乃至整个配置文件）失效**。

另外值域是 **[0, 10000]**，越界同样被拒。

**实现取的策略**：缓存价没填按 0 计（多数国产服务商不单独收缓存写入费，models.dev
也是不收才不写）；输入/输出价没填就整行不写 —— 那两个数编不得，宁可这个模型不显示费用。
所有值按 [0, 10000] 钳位。

### ② §2.2「换 slot 池会改变 config_hash」→ 证伪，不会

`canonical_hash` 只对 `providers + port` 取哈希，槽位是 `flatten_config` 时按顺序
现算的、根本不进配置文件。照原文实现的话，老用户升级后状态机显示「配置已生效」，
而 Claude Desktop 里写着的还是旧槽位 —— **永远不会有人去点那个「应用」**。

**实现补的机制**：`SLOT_POOL_VERSION` 常量进哈希输入；另存 `last_applied_pool`，
用来把「升级导致的 dirty」和「用户改了配置」区分开，前者显示本节原文那句提示语。
换槽位池时**必须同时改这个常量**（代码里已注明）。

### ③ 费率数据源：预设库手填人民币 → models.dev 运行时同步

价格表不自己维护，改为从 `https://models.dev/api.json` 同步。

models.dev 是社区维护的开源模型数据库（`anomalyco/models.dev`，MIT），数据以 TOML
存在仓库里、CI 编译成单个 api.json（4.5 MB，213 家服务商）。**`cost` 单位就是
USD/百万 token** —— 与 `inferenceModelPricing` 要的单位一致，这条路不经过汇率。

覆盖情况实测：8 家内置预设全部有对应条目，且有中国区变体（`moonshotai-cn`、
`zhipuai`、`minimax-cn`、`alibaba-coding-plan-cn`、`alibaba-token-plan-cn`、
`xiaomi-token-plan-cn`、`kimi-for-coding`、`deepseek`），与预设 URL 一一对得上。

**实现**：启动自动同步（6 小时阈值，先判是否要同步再决定下不下那 4.5 MB）+ 设置页
手动「立即同步」+ 开关。同步结果落进 `config.json` 的 `pricing_synced`，
手填的 `pricing` 永远优先。§五① 的汇率输入框保留，只服务于按人民币手填的场景。

**两条实测补的规则**：

- **订阅制服务商**：Kimi Code / 百炼 Token Plan / 小米 Token Plan / MiniMax Token Plan
  在 models.dev 里每个模型都是 0，但列的模型名常与用户填的对不上
  （`kimi-for-coding` 只列 `k3` / `kimi-for-coding`，用户填的是 `Kimi-k2.6`）。
  查不到就意味着这个槽位退回 Anthropic 官方价 —— 假账单。故加了「这家所有模型都是 0
  → 按 token 计费为 0」的判定。混价的（百炼 Coding Plan 有 qwen3.7-max 按量计费）不适用。
- **价格与上下文的回退规则必须分开**：上下文是**模型自身**的属性，同一个模型在哪家
  都是那么大，可以跨服务商借；价格是**服务商**的属性，同一个模型在不同家能差好几倍，
  绝不能借。

### ④ §3.7 兜底开关 → 已实测定稿：默认开，且确认有效

配置 `organizationInstructions` 为槽位映射说明后，在 Chat 里问「你是什么模型？」，
Kimi 回答：

> 我是 Kimi-k2.6，由月之暗面（Moonshot AI）开发。这里的 `claude-opus-5` 只是网关路由槽位名，
> 不代表我的真实身份。

即 `organizationInstructions` **压得住** §5.5.1 里那句第二人称的
"You are a Claude agent, built on Anthropic's Claude Agent SDK"。开关保持默认开。

**文案实现细节**：给的是**槽位映射表**而不是一句笼统的「你不是 Claude」——
模型自己知道「我是 claude-opus-5」（系统提示词里的 `ps()` 会写明 exact model ID），
给出映射它就能反推真实身份，多模型时同样成立。

`organizationInstructions` 的 schema 是 `D().trim().min(1).max(3e3)`：
**`min(1)` 意味着清空后必须删键**，写空串会被拒；`max` 按 UTF-16 码元计。

### ⑤ §3.5 补：`prefer1m` / `supports1m` 的前提是这个模型真有 1M

**预设里多数模型根本没有 1M**（models.dev 的 `limit.context`）：

| 模型 | 真实 context |
|---|---|
| kimi-k2.5 / kimi-k2.6 | 262144 |
| MiniMax-M2.7 | 204800 |
| glm-5.1 | 200000 |
| qwen3-coder-next | 262144 |
| deepseek-v4-pro / flash | 1000000 ✅ |
| qwen3.6-plus | 1000000 ✅ |

而 2.0 的 `addProviderFromPreset` 与「+ 添加模型」一律默认 `to_1m: "auto"`。

**后果是静默的**：日志实测代理**从没收到过带 `[1m]` 的请求** —— 印证 §六「后缀由引擎
剥离，改发 `anthropic-beta: context-1m-2025-08-07` 头」；而探针显示 Kimi 照收该头不误，
于是上游按自己的 262K 上限截断，用户以为有 1M。

**实现**：同步顺带取 `limit.context`；新增模型默认不开 1M；模型行上直接标出
「仅 256K」的告警色（不藏在折叠面板里）；apply 时把这类模型列进返回消息与日志。
判定门槛取 1000000（各家数字不一：Kimi 1048576 / 智谱 1000000）。
跨服务商取值时**取各家里最宽松的那个** —— 实测 26 家都列了 kimi-k2.6，多数写 262144，
但 routing-run 写 200000、hyper 写 262000、privatemode-ai 写 256000，要求一致就永远
得不出结论；取最大值意味着只有各家一致认为不到 1M 时才提示，宁可少提示。
**未知一律当作支持**，绝不因为 models.dev 缺一条数据就去质疑用户的设置。

### ⑥ §5.6 探针的缓存检测会假阴性

对 Kimi Code 实测：system 块 2412 token、6012 token、12012 token 三档，
`cache_creation_input_tokens` **恒为 0**；而 §5.5.5 的真实 Chat 会话里
有 `cache_read_input_tokens: 9728`。说明这家只是不吃这种合成探测形态，
不代表它不支持缓存。

**实现**：结论文案从「每轮全量重算，成本和延迟都会明显上升」降级为
「本次探测未观察到缓存命中（合成请求可能触发不了上游的缓存条件，真实多轮会话里
未必如此）」。两次缓存请求之间补了 2s 间隔（缓存建立是异步的）。

### ⑦ §3.11 整流器的顺序坑

budget（§3.11.2）与 thinking 块（§3.11.3）的报错**同样是 400**，而请求里往往还带着
`output_config`。若让 §3.11.1 的投机分支先命中，就会去删一个跟错误无关的字段，
白白浪费一次整流额度、也治不好原问题。

**实现**：选择逻辑抽成 `pick_rectifier`，顺序为「错误消息明确点名的整流器 →
effort 的投机分支排最后」，并单独钉了测试。

### ⑧ 其它就地核实的 schema（免得以后再翻 asar）

| 键 | schema | 备注 |
|---|---|---|
| `inferenceStreamIdleTimeoutSec` | `Un().int().min(300).max(1800).optional()` | 1800 是合法上界 |
| `anthropicFamilyTier` | `Rn(Ba).optional()`，`Ba = ["sonnet","opus","haiku","fable","mythos"]` | 先 trim + 小写；写枚举外的值会被拒 |
| `isFamilyDefault` | `k().optional()`，show 谓词 `!!e.anthropicFamilyTier` | |
| `prefer1m` | `k().optional()`，show 谓词 `!!e.supports1m` | |
| `egressProxyUrl` / `egressProxyPacUrl` | `Ho({allowHttp:!0}).optional()`，1.44121.1 | **PAC 设了就压过普通代理**，原文 "the PAC file wins and this key is ignored" |
| `organizationInstructions` | `D().trim().min(1).max(3e3)`，1.37937.0 | |

**Workspace 那一页的开关**（2026-09-15，1.49585.0 核实；「一键使用 Winhao 的配置」用）。
全部是 `k().optional()` 布尔。**写进配置文件的是 flatKey，不一定等于界面上的字段名**：

| 界面上 | flatKey | 默认 | 起始版本 |
|---|---|---|---|
| Cowork | `coworkTabEnabled` | 开 | 1.9659.0 |
| Code | `isClaudeCodeForDesktopEnabled` | 开 | 1.2581.0 |
| Chat | `chatTabEnabled` | 无（不写就是关） | 1.13576.0 |
| Block reads outside working directories | `blockReadsOutsideWorkingDirectories` | 关 | 1.46388.1 |
| Allow Auto mode | `autoModeEnabled` | 关 | 1.10628.0 |
| Disable bypass permissions mode | `disableBypassPermissionsMode` | 关 | 1.46388.1 |
| Disable bundled skills and workflows | `disableBundledSkills` | 关 | 1.15962.0 |
| Allow user-created skills | `skillCreationEnabled` | 开 | 1.25927.0 |
| Allow user-added plugin marketplaces | `userPluginMarketplacesEnabled` | 开 | 1.37937.0 |
| Allow user-added plugins | `userPluginUploadsEnabled` | 开 | 1.37937.0 |
| Disable Claude.ai sign-in | **`disableDeploymentModeChooser`** | 关 | 1.3834.0 |
| Disable claude:// deep-link handling | **`disableDeepLinkRegistration`** | 关 | 1.6889.0 |
| Skip WebFetch domain check | `skipWebFetchPreflight` | 关 | 1.37937.0 |
| Enable tool search | `toolSearchEnabled` | 关 | 1.21459.0 |
| Advanced file analysis | **`chatAdvancedFileAnalysisEnabled`** | 关 | 1.14271.0 |

几条从 `description.long` 里读到的机制细节：

- **Skip WebFetch domain check**：Code 里抓网页前先问 `api.anthropic.com` 这个域名在不在黑名单，
  问不到就拒绝抓取（"Unable to verify if domain … is safe to fetch"）；连得上时每个抓取的域名都会发过去
- **Enable tool search**：gateway 下会给请求加 `tool-search-tool-2025-10-19` beta、deferred tool loading、
  `tool_reference` 内容块，原文 "If your endpoint does not accept the request shape it then receives, requests fail with HTTP 400"
- **Allow Auto mode**：3p 下没开这个键时，桌面端给 Code 会话传 `disableAutoMode: "disable"`；
  开了之后每个操作先过安全分类器，只对它判为有风险的弹确认
- **Allowed egress hosts**（`coworkEgressAllowedHosts`）：不设时沙箱只能连推理地址，装包、抓网页 403；
  `*` 关掉网络沙箱，但网页抓取仍挡内网地址
- 遥测两项 `disableEssentialTelemetry` / `disableNonessentialTelemetry` 在 3p 下默认就是开（已屏蔽），不用写

`claude-opus-5` 在本机 1.46388.3 的 `Vwt` 表里确认带
`effortLevels:["low","medium","high","xhigh","max"], recommended:"high", modes:["auto"]`
**外加一个 §1.1 没记的 `disallowThinkingDisabled:!0`** —— 意味着这些槽位上「关闭思考」
可能会被桌面端直接拒掉，值得后续验证。
而 `claude-3-opus-latest` 在整个 app.asar 里出现 **0 次**，坐实 §1.1 的判断。

### ⑨ §5.5.4 / §5.5.5 已补实现（原先没排进 A–E）

- **标题生成省思考**：按首条 user 消息的开头
  （`You are coming up with a succinct title`）识别，注入
  `output_config:{effort:"low"} + thinking:{type:"disabled"}`。
  刻意不看 `max_tokens=200` —— 那是实现细节，容易随版本变。
  桌面端已明确指定 effort 时不动（与 §3.10 同一条原则）。**默认开。**
- **健康检查本地应答**：`max_tokens=1` + 正文一个点 + 无 system + 无 tools，
  四条全中才算。**默认关** —— 只省 8 个 token，却会让探活在上游已经挂掉时依然显示
  正常，掩盖真实故障的代价远大于这点开销。判定也刻意从严：宁可漏认，
  也不能把用户真正的请求本地短路掉。
- 两者都在设置页有开关，命中时请求日志标「已优化：…」。
- **`modelDiscoveryEnabled: false`** 已随网关键一起写入。

至此设计文档中描述过的改动全部实现完毕。

### ⑬ 设置页减负：删掉 7 项（2026-09-10，用户拍板）

2.1 一口气把设置页从 5 项加到 13 项。按「**用户要基于什么知识、在什么情况下做这个
决定**」逐条评估，答案是「永远不需要动」或「得懂 ModelLink 内部机制才能决定」的一律砍掉：

| 项 | 处理 | 理由 |
|---|---|---|
| 健康检查本地应答 | **功能整个删** | 只省 8 个 token，代价是上游挂了也显示正常 —— 掩盖故障的功能不该存在 |
| 标题生成省思考 | 删开关，**行为保留** | 开着永远更好（省钱、标题质量无变化），用户没有理由关，那就不该问 |
| 未映射槽位回落（兼容模式） | **删** | 给「依赖旧行为的用户」的台阶，但 2.1 换了槽位池，旧行为本就不存在 |
| 流式心跳间隔 | 删设置，**固定 15s** | 用户没有依据判断该填几秒；这个值也没有副作用（只在沉默时写一行 SSE 注释） |
| 人民币兑美元汇率 + 币种切换 | **整套删，费率一律美元** | `inferenceModelPricing` 与 models.dev 的 `cost` 本来都是 USD/百万 token，换算这一层是自找的 |
| Claude 出网代理 | **删** | 真需求但极少数人用，说明有 5 行全是约束 |
| 组织级指令输入框 + 槽位映射开关 | 删控件，**槽位映射行为保留、常开** | 输入框是可选高级功能；而槽位映射说明实测有效（Kimi 正确答「我是 Kimi-k2.6」），删了它 Chat 里会变回自称 Claude，所以留行为不留决策 |

**结果：13 个控件 → 6 个**（外观 / 代理端口 / 自动同步费率 / 开机自启 / 软件更新 / 关于，
外加两行只读信息）。

**顺带记一条自我批评**：这些设置项的说明文字大量出现实现细节 ——
「实测花掉 88 个思考 token」「1 token 的探活请求」「系统提示词里有一句
You are a Claude agent」。那是写给开发者看的，用户做决定并不需要知道。

### ⑭ 服务商编辑器再减负：删掉三个按钮（2026-09-10，用户拍板）

| 按钮 | 连带删掉 | 影响 |
|---|---|---|
| **深度探测** | 整个 `probe.rs` + `probe_provider` 命令 + 结果面板 | 失去「一次问清这家支持到什么程度」的信息展示。连通性仍由「测试连接」覆盖；effort 支持与否仍由 §3.11.1 的整流器在运行时自学 |
| **费率** | `ModelEntry.pricing`（手填）+ 费率面板 | 费率**全部来自 models.dev 同步**，没有手填覆盖入口了 |
| **层级** | `prefer_1m` / `family_tier` / `family_default` + 层级面板 | §3.5 的三个模型条目字段不再写入，`inferenceModels` 回到 `{name, supports1m, labelOverride}` 三项 |

**顺带收紧的一处**（因为手填入口没了）：`inferenceModelPricingEnabled` 原本是
「有任意一行费率就开」。手填补救手段消失后，「部分模型有同步价、部分没有」的情况
会让没价的那条退回 Anthropic 官方价 —— 半真半假的账单比不显示费用更糟。
改为**所有已路由模型都有价才开**，否则整张表不写。

`scripts/probe-provider.py` 保留 —— 它仍是接入新服务商时先跑一遍的工具，
只是不再内置进界面。

### ⑮ 模型名补全改用 models.dev 实时清单（2026-09-10，用户提议）

**问题**：预设里的模型清单写死在 `presets.ts`，只能靠发版更新。对照 models.dev
发现 16 个预设模型里有 6 个对不上，其中 **Kimi Code 那条已落后两代** ——
预设写的是 `Kimi-k2.6`（2026-04-21），而该服务商现在提供的是 `k3`（2026-07-16），
中间还隔着 `kimi-k2.7-code`（06-12）。用户对此完全无感。

models.dev 的更新很勤：最近提交全是当天的 `chore(sync): update XXX model catalog`，
由 CI 自动从各家目录同步，不靠人工提 PR。

**做法**：模型名输入框的补全列表（datalist）改成从 models.dev 取，按当前服务商
对应的条目实时给，**按发布日期新→旧排序**（用户第一件事是挑当前能用的模型）。
预设里那份写死的清单降级为「同步不到时的兜底」。

**刻意只做补全、不做强制替换**：models.dev 的 `id` 是 AI SDK 的标识符，
不保证等于服务商 API 接受的 model 字符串；而 Kimi Code 这类端点是**静默回落**的
（发 `zzz-nonexistent` 也返回 200），ID 错了不会报错、只会悄悄给另一个模型。
作为建议列表这个风险可接受，作为自动替换则不可接受。老模型用户照样能手输任意名字。

**两处工程细节**：

- 模型 ID 列表与费率走**同一次抓取**（`fetch_catalog` 一并返回），零额外请求。
  但费率只收有 `cost` 的条目，补全列表要列全「这家现在提供什么」，
  所以是两个独立的解析函数。
- **只留我们认得的 10 家**。api.json 有 213 家，全存进 `config.json` 实测会让它
  从 600 字节涨到 **238 KB** —— 而那个文件每次编辑都要重写。过滤后 5.3 KB。
  另有一条测试钉住「`provider_id_for_url` 的每个值都在 `known_provider_ids` 里」，
  两处漂移会让某家的补全列表静默消失。

### ⑯ 模型清单三级兜底：发版快照（2026-09-10，用户提议）

⑮ 把补全改成运行时拉 models.dev 之后仍有一个缺口：**用户首次打开、断网、
或 models.dev 不可达时拿不到任何清单**，而那正是他最需要挑模型的时刻。
（这个缺口在开发中真实触发过一次：某次启动同步失败，配置里 `models_dev_models` 为空。）

**做法**：`npm run sync-models` 在发版前抓一次 models.dev，生成
`src/lib/modelsSnapshot.ts` 并提交进仓库，打包进安装包。补全清单三级兜底：

1. 运行时同步来的（最新）
2. **发版快照**（`npm run sync-models` 生成，4.8 KB）
3. 手写在预设里的那份（最后兜底 —— 会过期，实测落后过两代）

生成脚本与后端 `parse_model_ids` 用同一套排序（发布日期新→旧），
服务商列表与 `known_provider_ids()` 对齐。发版步骤已写进 `SIGNING.md`；
快照文件的 diff 顺带能看清各家这段时间新增/下线了哪些模型。

**一处刻意不做**：不用「最新的模型」自动当预设默认值。DeepSeek 当前最新的是
`deepseek-v4-flash-vision-exp`（实验性视觉模型），自动填进去会很糟。
预设点击时创建哪个模型仍由人工挑选的 `Preset.models` 决定。

### ⑫ Kimi 官方文档核实：只有 3 档，且换档会破前缀缓存

`api.moonshot.cn/anthropic` 的 Messages API 文档里 `output_config.effort` 写明：

> 推理强度，支持 **low、high、max**，默认 max。
> **切换档位会破坏前缀缓存命中，建议在会话开始前确定。**

两条修正与一条新事实：

1. **探针「五档全部接受」是 over-claim。** Kimi 只有三档，`medium` / `xhigh` 能返回
   200 是因为**网关侧映射**（§五① 那张表：medium→high、xhigh→max），不是真有五个
   不同强度。措辞已改为「五个档位均未被拒」，并明说未必真的生效。
2. **「未被拒」证明不了「已生效」。** 探针原本只看状态码，分不出「照做」和「收下后
   扔掉」—— 和模型名静默回落是同一类问题。唯一可靠的判据是**思考量随档位变化**。
   实测（`api.kimi.com/coding/`，同一道推理题，各一次）：

   | effort | thinking_tokens | output | 耗时 |
   |---|---|---|---|
   | 不发 | 1582 | 1993 | 61s |
   | low | 1337 | 1641 | 50s |
   | max | **3616** | 3976 | 112s |

   low → max 思考量 2.7 倍 —— 这才是「真的生效」的实证。
   探针现在会读回 `usage.output_tokens_details.thinking_tokens`，**只有当各档确实
   拉开差距时**才把结论升级为「已确认真实生效」，否则如实说「没能确认」。
3. **换档破前缀缓存**（此前无人提及）。影响面：用户在 Claude 选择器里中途换档，
   Kimi 侧缓存失效、下一轮全量重算，而桌面端不会提示。我们的 effort 整流器在被拒时
   摘掉 `output_config` 也算换档，但那是错误路径。标题生成优化**不受影响** ——
   它是独立的一次性请求，不与会话共享前缀。

**~~一处对不上，待复测~~ → 已由 Kimi 的档位映射表解答**（用户提供，2026-09-10）：

| Claude Code 档位 | K3 实际档位 |
|---|---|
| low | low |
| medium | high（推荐） |
| high | high（推荐） |
| xhigh | max |
| max | max |
| **未设置（默认）** | **high** |

所以「默认 max」那句（Messages API 参数说明里）与实际不符，**未设置时走的是 high**。
这与实测吻合：不发 effort 得 1582，正落在 low(1337) 与 max(3616) 之间。无需复测。

两条对使用者有实际影响的推论：

- **Claude 的 5 档在 Kimi 上只有 3 种真实效果**：medium 与 high 同为 high，
  xhigh 与 max 同为 max。用户在选择器里 medium ↔ high 之间切换，行为完全不变。
- 结合上面那条「切换档位会破坏前缀缓存命中」—— 这类切换**白白损失缓存、换来零收益**。
  桌面端不会提示，我们目前也无法在它的选择器上加注（那是 app 自己的 UI）。

这同时印证了 §五① 的结论：**默认透传，ModelLink 不该自作主张映射**。
映射是服务商网关的事，且 Kimi 的映射是「质量优先」（medium→high 是往上抬，
不是线性降级），代理插手只会降低质量。

**由此撤回一条先前的建议**：原本打算把服务商级下拉统一补齐成 5 档，靠整流器兜底。
既然各家档位数本就不同（Kimi 3 档），统一补 5 档反而让用户以为有 5 个不同强度。
改为**按官方文档/探测结果逐家填**：Kimi 两家已按文档补成 low/high/max，
其余 6 家没有依据，保持原样。

### ⑩ 兜底注入不再写 `budget_tokens: 8192`

§3.10 原本要求「仅在兜底注入时保留」这个值。实施后回头看，保留的理由不成立：

- 8192 在 v1 代码里是个**没有出处的裸字面量**，全仓库无注释、无文档说明
- 它与 `output_config.effort` 语义重叠（同时写两条腿）
- 对 1M 上下文模型明显偏小

改为**照抄桌面端实际发的形态**（§5.5.2 抓包）：`output_config:{effort}` +
`thinking:{"type":"adaptive"}`。好处是零编造数字 —— 「想多久」交回给上游自己定，
且与桌面端在有选择器的槽位上发的东西完全一致。

另：按 app.asar 里的 CLI 映射，`thinking:{"type":"enabled"}` **不带 budget** 时（`enabled` 无 budgetTokens → `--thinking adaptive`）同样等价于自适应。

**顺带扩了 §3.11.2 一条**：兜底改发 adaptive 后，多了一种上游拒收的可能
（不认 adaptive 这个形态）。budget 整流器原本对 adaptive 请求一律不动，
现在区分两种情形 —— 抱怨 budget 下限时仍然不动（改了反而报新错），
抱怨 adaptive 本身时则换成显式的 `enabled + 32000`。

### ⑪ 服务商级「默认推理强度」按槽位隐藏

换池后前 6 个槽位在 Claude 里有原生选择器，桌面端每次都发 effort，服务商级设置
完全不参与 —— 这个下拉对多数用户的多数模型已经不生效，留着只会让人困惑
（实际发生过：使用者反复追问它到底管什么）。

改为只在该服务商**有模型落在没有选择器的槽位上**时才显示：
`claude-sonnet-4-5` / `claude-haiku-4-5`（`Vwt` 里没有 `effortLevels`）
与 `claude-ml-*` 溢出层。隐藏但配置里仍有旧值时给一行提示 + 「清除」按钮
—— 那个值仍对不带 effort 的内部请求生效，不能一声不吭地藏掉。

我们走的是透传优先、被拒才整流：不按服务商硬编码丢掉桌面端发来的思考设置。

**遗留**：下拉的选项集仍是 2.0 时代的 —— 8 家预设里 7 家只有「默认 / 关闭思考」，
拿不到 low/medium/high/xhigh/max。而它现在只出现在「唯一控制入口」的场合，
限制的代价更大了。待定：统一补齐 5 档（有整流器兜底），或按探针结果逐家填。
