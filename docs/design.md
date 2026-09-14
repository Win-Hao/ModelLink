> ⚠️ **本文档已作废（2026-09-14）。**
> 2.2 的界面规格见 **[`design-2.2.md`](design-2.2.md)** —— 方向 O「暖白精工」，
> 窗口 1050×700、取消左侧栏改顶部 tab、水印移除、字号下限抬到 11px。
> 本文保留为 2.0 的历史记录，**不要按它实现**。

---

# ModelLink 2.0 界面设计规格（已冻结）

> 状态：**2026-07-13 定稿**，用户选定方案 v2「侧栏导航 + 模型链路板」。前端实现以本文档为准，不要自行发挥设计。
> 视觉参照：`docs/design-proposal.html`（v2 高保真 mockup，像素细节以它为准）。`docs/design-proposal-v3.html` 为落选方案存档，勿参考。

## 1. 窗口与骨架

- 窗口 **880 × 640**，最小 800 × 560，可调大小，启动居中。
- macOS：隐藏标题栏（`titleBarStyle: "Overlay"` + `hiddenTitle: true`），红绿灯悬浮在侧栏左上；侧栏顶部约 44px 高度为拖拽区（`data-tauri-drag-region`）。
- Windows：标准标题栏（不做自绘）。
- 结构：左侧 **178px 固定侧栏** + 右侧页面区。页面区页头顶部留 46px（macOS 与红绿灯对齐；Windows 可统一沿用，视觉无碍）。

## 2. 色板（shadcn/ui CSS variables，new-york 风格 + CSS variables 主题）

| token | Light | Dark | 用途 |
|---|---|---|---|
| `--background` | `#FAF9F5` | `#1B1D21` | 内容区底（暖纸 / 暖炭） |
| sidebar 底 | `#F1EEE6` | `#17181C` | 侧栏（用 shadcn `--sidebar` 系变量） |
| `--card` | `#FFFFFF` | `#24262C` | 卡片 |
| `--foreground` | `#26231D` | `#EDEBE3` | 主文字 |
| `--muted-foreground` | `#6F6A60` | `#A5A094` | 次级文字 |
| faint（自定义） | `#A39D91` | `#716C62` | 三级文字（区块标签、槽位名、说明） |
| `--border` / `--input` 边框 | `rgba(38,35,29,.11)` | `rgba(255,255,255,.10)` | 边框 |
| 输入框底 | `#FFFFFF` | `#1F2126` | input 背景 |
| `--primary` | `#D97757` | `#D97757` | 品牌陶土色，双主题同值；hover 浅 `#C4684A` / 深 `#E08A6D` |
| primary-soft | `rgba(217,119,87,.11)` | `rgba(217,119,87,.16)` | 头像底、激活 tint |
| success | `#26836C`（soft `rgba(38,131,108,.12)`） | `#4DB8A0`（soft `rgba(77,184,160,.15)`） | 运行中、连通、200、已生效 |
| warning | `#A97A1E`（soft `rgba(169,122,30,.12)`） | `#D4A843` | dirty 未应用态专用 |
| `--destructive` | `#C9534C` | `#E0685F` | 删除、错误状态码 |

主题三态（亮 / 暗 / 跟随系统）：`.dark` class + localStorage 持久化；「跟随系统」用 `matchMedia('(prefers-color-scheme: dark)')` 监听切换。

## 3. 字体与字号

- 字体族：`Outfit`（拉丁数字，**本地打包 woff2 子集**，不走 CDN）→ 回退 `-apple-system, "PingFang SC", "Microsoft YaHei"`。mono：`ui-monospace, "SF Mono", Menlo, Consolas`。
- **所有技术值一律 mono**：槽位名、模型名、URL、端口、时间戳、计数（配 `font-variant-numeric: tabular-nums`）。

| 角色 | 字号/字重 | 用途 |
|---|---|---|
| 页面标题 | 19px / 650 | 每页一个，字距 -0.01em |
| 侧栏品牌 | 14.5px / 650 | 字标 + 20px 圆角 6px 的 primary logo 方块 |
| 侧栏导航 | 12.5px / 500（激活 600） | 四个导航项 |
| 卡片区块标签 | 11px / 600 | 「模型链路」「最近请求」等，字距 +0.08em |
| 正文/表单 | 13px / 400 | 说明文字、设置项；输入框内容 12px mono |
| 链路板槽位 | 11.5px mono | muted 色 |
| 链路板真实模型 | 12px mono / 600 | 主文字色 |
| 说明/提示 | 10–11px | 槽位行内提示 10 mono、端口 10 mono、水印 9.5 |

## 4. 布局 token

| token | 值 |
|---|---|
| 圆角 | 卡片 12px；输入框/按钮 9px；清单行 10px；chip/日志标签 5–6px；徽章胶囊 999 |
| 间距 | 8pt 网格；页面内边距 24px；卡片间 14px；表单行距 12px；链路板行高 ~38px |
| 控件高度 | 主按钮 35px；输入框 33px；次级按钮/Select 29px |

## 5. 侧栏规格

- 品牌区：logo 方块 + 「ModelLink」。
- 导航四项（lucide 图标 14px）：**概览**（Zap）、**服务商**（Layers）、**请求日志**（ScrollText）、**设置**（SlidersHorizontal）。激活态：card 底 + 600 字重 + 轻阴影（深色主题无阴影）。
- dirty 时「概览」项右侧显示 6px 琥珀圆点（全局提醒）。
- 底部常驻两块：
  - 状态块（上边框分隔）：绿点 + 「代理运行中」11.5/600 success 色，下行端口 `127.0.0.1:<端口>` mono 10（端口动态，默认 5678）。端口被占时变 warning 色点 + 「代理未运行（端口被占）」（2026-07-14 用户新增）。
  - 水印块：两行 9.5px faint 色——「Winhao学AI · 抖音搜索同名」/「免费软件 · 不可商业化」（防篡改护栏见 §10）。

## 6. 页面规格

### 6.1 概览

- 页头：标题「概览」+ 副行「N 个模型 · M 个服务商 · 上次应用 `<时间>`」；右侧为应用状态区（§8）。
- **模型链路板卡**（主视觉）：头部标签「模型链路」+ 右侧 `n / 8 槽位` mono 计数。行结构：`槽位名（mono, muted, 宽 212 截断）→ 箭头图标 → 真实模型（mono/600）+ [1M 徽章] + 服务商 tag（15px 圆头像 + 名称，胶囊）`。头像用品牌 logo（@lobehub/icons-static-svg，白底圆，Kimi 黑底；自定义回退首字母，2026-07-14 用户新增）。行间 1px 分隔线，悬停整行浅底。**行可点击**：跳转服务商页并选中对应服务商。已用槽位下方一条虚线提示行：「其余 X 个槽位空闲 · 在「服务商」页添加模型后自动接入」。
- **最近请求卡**：最新 2 条日志（时间 mono · 状态点 · 模型 · 思考标签 · 状态码）+ 头部右侧「查看全部 →」跳日志页。
- 1M 徽章样式：9px/700 primary 色描边小方签。

### 6.2 服务商

- 页头：标题「服务商」+ 副行「n / 8 个模型槽位已使用」；右侧 dirty 时显示「应用」胶囊（warning-soft 底胶囊内嵌 primary 小圆钮「应用」），点击即执行 apply，不必回概览页。
- **双栏**：左列 196px 服务商清单 + 右侧编辑器卡。
  - 清单项：头像（品牌 logo，自定义回退首字母 primary-soft 底；2026-07-14 用户新增）+ 名称 12.5/600 + 副行「n 个模型 · 已连通」10.5 faint + 状态；激活项边框 `rgba(217,119,87,.5)`。列表底部虚线「+ 添加服务商」。
  - 编辑器：
    - 「API 地址」「API 密钥」两字段（上下两行，各占整行；2026-07-14 用户调整，原 1.2fr:1fr 双列太挤）；密钥为密码框 + 眼睛图标切换明文。
    - 模型区标签行：「模型 · 右侧为 Claude 中显示的名称」+ 右侧「测试连接」outline 按钮，结果弹 Sonner toast（§9；2026-07-14 用户调整，原 inline 结果）。
    - 模型行：模型名输入（mono，带预设模型建议——datalist 或 Combobox）+ 1M Switch + 行内槽位提示 `→ claude-3-…`（mono 10，宽 196 截断）+ 删除 X。
    - 「+ 添加模型」虚线按钮。
    - 底行：左「推理强度」Select（选项按服务商预设约束：默认（不干预）/ 关闭思考 / 标准 (high) / 深度 (max)）；右「删除服务商」ghost 红字 → AlertDialog 二次确认。
- 空状态：无任何服务商时，双栏隐藏，整个区域铺预设网格（§7）。

### 6.3 请求日志

- 页头：标题「请求日志」+ 副行「保留最近 100 条 · 页面停留时每 2 秒自动刷新」；右侧「刷新」outline 按钮。
- 表格卡整页：行 = `时间（mono, faint）· 状态点（200 绿 / 其他红）· 模型名（mono/600, 弹性截断）· 思考标签 chip（深度/默认/思考关…）· 状态码（200 success 色 / 其他 destructive 色）`。
- 卡片底部诊断提示（faint 小字）：「诊断建议：如果这里长期空白而 Claude 无法对话，请检查是否已点「应用到 Claude Desktop」。」

### 6.4 设置

- 单卡片平铺内容区宽度（2026-07-14 用户调整，原 max-width 470），五行（行间分隔线）：
  1. 外观 —— segmented 三态（亮色 / 深色 / 跟随系统），Tabs 定制成 segmented 样式。
  2. 代理端口（副行「修改后立即生效，需重新应用到 Claude Desktop」）—— mono 数字输入，blur/Enter 提交热切换（2026-07-14 用户新增；默认 5678，范围 1024–65535）。
  3. 开机自启（副行「登录时自动启动代理」）—— Switch。
  4. 软件更新（副行「当前 x.y.z · 已是最新版本」或「发现新版本 vX」）——「检查更新」outline 按钮。
  5. 关于（副行「ModelLink by Winhao学AI · 免费软件」）—— 「GitHub ↗」外链。

## 7. 空状态与引导（首启）

- 零服务商时概览页变引导页：页头标题「选择一个服务商开始」+ 副行「配置 API 密钥后，一键接入 Claude Desktop」；链路板位置换成 **3×3 预设网格**：8 个预设（DeepSeek / Kimi Code / Kimi 开放平台 / MiniMax / 百炼 Coding / 百炼 Token / GLM（智谱）/ mimo，数据与 thinkingOptions 约束沿用 v1 `ui.html` 的 PRESETS）+ 第 9 格虚线「自定义」。tile = 品牌 logo 头像（自定义为 ?）+ 名称 12/600 + 域名 mono 9px。
- 点击预设 → 创建服务商（预填 URL 与预设模型）→ 跳服务商页选中它并聚焦密钥输入框。
- 已有服务商后，「+ 添加服务商」打开同一网格的 Dialog 形态。

## 8. 应用状态机（核心交互）

| 态 | 判定 | UI |
|---|---|---|
| clean | 当前配置 hash == last_applied_hash | 概览页头右侧：绿勾 +「配置已生效」11.5 + outline「重新应用」 |
| dirty | hash 不等 | 琥珀点 +「配置已修改，尚未应用」+ primary「应用到 Claude Desktop」；同时侧栏「概览」琥珀点亮起、服务商页头出现「应用」胶囊（三处同源） |
| applying | apply 进行中 | 按钮内 spinner +「正在重启 Claude…」，全局禁止再次触发 |
| error | apply 失败 | 红字错误摘要 + 「重试」按钮 + toast 详情 |

- hash = 规范化 JSON（稳定键序）的摘要；`last_applied_hash` 持久化（存 config.json 内新增字段），重启不丢。
- **自动保存**：任何编辑 400ms 防抖写盘；无「保存」按钮；写盘失败 Sonner toast 报错。
- apply 成功：更新 last_applied_hash，三处 dirty 提示熄灭，链路板各行依次做一次 300ms 绿色微闪。

## 9. 约束与反馈

- **8 槽位上限**：`n / 8` 常驻（概览链路板头 + 服务商页头副行）；满槽时「添加模型」禁用 + Tooltip「所有服务商的模型总数最多 8 个」。
- **测试连接**：点击后按钮 loading（Loader2 spin）→ 结果弹 Sonner toast：成功绿「连接成功 (HTTP 200)」，失败红（后端话术）。（2026-07-14 用户调整，原为 inline 结果 6 秒淡出。）
- **端口占用**：启动时端口被占 → 原生提示框（沿用 v1 话术 + 「可在设置页更换端口」），**不退出**：侧栏状态块转「代理未运行（端口被占）」，设置页换端口即热恢复。（2026-07-14 用户调整，原为弹窗后退出。）

## 10. 水印（防篡改，保护强度不降）

- 呈现：侧栏底部两行（§5）；关于页含完整免费声明。
- 实现：移植 v1 `ui.html` 的护栏逻辑为 React hook（`useWatermark`）：MutationObserver 监视删除/样式篡改并恢复、周期性完整性检查、`Element.prototype.remove` 等拦截。文案与签名机制不变。

## 11. shadcn/ui 组件映射与图标

| 组件 | 用途 |
|---|---|
| Button / Input / Label / Card / Separator | 全局；虚线添加按钮用自定义 variant |
| Switch / Select / Tabs | 1M、开机自启；推理强度；外观三态 segmented |
| Badge | 运行中徽章、1M、思考标签、服务商 tag |
| Dialog / AlertDialog | 添加服务商预设网格、更新弹窗（UpdateModal 移植 ClaudeCN）；删除服务商确认 |
| Sonner / Tooltip / ScrollArea | toast；满槽提示等；链路板与日志滚动 |
| Progress / Skeleton | 更新下载进度；启动时链路板占位 |

lucide 图标：`Zap, Layers, ScrollText, SlidersHorizontal, Plus, Trash2, X, ChevronDown, ChevronRight, Eye, EyeOff, Check, ExternalLink, RefreshCw, Loader2, ArrowRight`。

## 12. 动效

- 页面切换：160ms 淡入 + 6px y 位移（framer-motion `AnimatePresence mode="wait"`，对齐 ClaudeCN 的 Fade）。
- apply 成功后的链路板绿闪（§8）是唯一表演性动效。
- 其余零装饰动效；全部动效尊重 `prefers-reduced-motion`。

## 13. 关键文案表（保持逐字一致）

- 侧栏：`概览` `服务商` `请求日志` `设置`；状态块 `代理运行中` + `127.0.0.1:5678`；水印 `Winhao学AI · 抖音搜索同名` / `免费软件 · 不可商业化`
- 状态机：`应用到 Claude Desktop` / `配置已修改，尚未应用` / `配置已生效` / `重新应用` / `正在重启 Claude…`
- 概览：`模型链路` / `n / 8 槽位` / `其余 X 个槽位空闲 · 在「服务商」页添加模型后自动接入` / `最近请求` / `查看全部 →`
- 服务商：`n / 8 个模型槽位已使用` / `+ 添加服务商` / `+ 添加模型` / `API 地址` / `API 密钥` / `模型 · 右侧为 Claude 中显示的名称` / `测试连接` / `连接成功 (HTTP 200)` / `推理强度` / `删除服务商`
- 日志：`保留最近 100 条 · 页面停留时每 2 秒自动刷新` / 诊断提示见 §6.3
- 引导：`选择一个服务商开始` / `配置 API 密钥后，一键接入 Claude Desktop` / `自定义`（副行 `手动填写地址`）
- 设置：`外观`（`亮色/深色/跟随系统`）/ `开机自启`（`登录时自动启动代理`）/ `软件更新`（`当前 x.y.z · 已是最新版本` / `检查更新`）/ `关于`（`ModelLink by Winhao学AI · 免费软件`）
