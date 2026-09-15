# 2.2 界面重做 · 交接

> 设计阶段已完成（2026-09-14）。本文是实施阶段的起点，新 session 从这里读起。

## 先读什么

1. **`docs/design-2.2.md`** —— 界面规格，实现以它为准。`docs/design.md`（2.0）已作废。
2. **`PRODUCT.md`** —— 产品事实：用户是谁、五条不可违背的原则。
3. `CLAUDE.md` —— 工程约束（回归套件、app.asar 核实、diff 网关配置）。

## 已定的决定（不要重新讨论）

| 项 | 决定 |
|---|---|
| 视觉方向 | **O「暖白精工」**，深色模式同结构换变量 |
| 窗口 | **1050 × 700（3:2）**，最小 920 × 620（原 880×640） |
| 导航 | **取消左侧栏**，改顶部居中胶囊 tab |
| 水印 | **删除**。`useWatermark.ts` / `.ml-wm` 样式 / `Sidebar.tsx` 宿主 div 全删 |
| 概览页排序 | **扁平按槽位，不分组**。排序逻辑已正确，不用改 |
| 字号下限 | **11px**，2.0 的 9.5 / 10px 全部废止 |
| 字体 | Archivo（拉丁）+ PingFang SC（中文）+ Spline Sans Mono（技术值），废弃 Outfit |

## 实施顺序

- [x] **1. 设计系统层** —— `src/index.css` 换色板 / 字阶 / 圆角 token；
      打包 Archivo + Spline Sans Mono 的 woff2 子集（放 `src/assets/fonts/`）。
- [x] **2. 骨架** —— 删 `components/Sidebar.tsx` 与 `lib/useWatermark.ts`；
      `App.tsx` 改顶栏 tab 布局；`src-tauri/tauri.conf.json` 窗口改 1050×700 / min 920×620。
      ⚠️ macOS 拖拽区：顶栏整条 `data-tauri-drag-region`，但 tab 与右侧按钮要 `no-drag`。
- [x] **3. 概览页** —— `pages/OverviewPage.tsx` + 新的链路表组件（替换 `LinkBoard.tsx`）。
      `RecentRequests.tsx` 并入页脚一行。
- [x] **4. 服务商页** —— `pages/ProvidersPage.tsx` 改 tab 条；`ProviderEditor.tsx` 重做：
      面板头（含常驻连通状态）、等宽两栏字段、**模型下拉选择器**（用 `models_dev_models`）、
      **1M 开关自判断**（用 `claims1mItDoesNotHave()`）、`⋯` 菜单放删除。
- [x] **5. 设置页** —— `pages/SettingsPage.tsx` 分三组 + 新增「一键排查」。
      排查项需要新的 IPC：端口占用、Claude Desktop 配置是否写入、各家密钥连通、dirty 状态。
- [x] **6. 日志页** —— ⚠️ **这步要先改 Rust**。
      `LogEntry` 新增三个字段：请求耗时、token（input/output）、费用。
      `proxy.rs` 记录耗时与上游 `usage`；费用用 `config.rs` 的费率表算。
      **费率不齐时整列不显示**（`PRODUCT.md` 原则一），绝不估算。
      改完必须跑回归套件（`gui-tauri/regression/run.sh`），表外用例逐字节一致。
- [x] **7. 首启引导** —— `PresetGrid.tsx` 上方加三步说明条。

## 每步做完

```bash
cd gui-tauri && npm run check          # tsc
cd src-tauri && cargo clippy --all-targets   # 必须零告警
```

改到第 2 步之后就可以 `npm run tauri dev` 看真实效果了。

按 CLAUDE.md 第三条：**跑测试 → 应用一次并 diff 网关配置 → 再看结论**。
第 6 步动了 `proxy.rs`，回归套件是必须的。

## 实施记录（2026-09-14）

七步都已落地，tsc / clippy 零告警 / `cargo test`（含 `--ignored`）全过，回归套件 ALL GREEN（表外用例与 v1 逐字节一致）。下面是规格没写、
实施时补的决定，接手前先看。

**后端新增（都没动转发出去的字节）**

| 什么 | 为什么 |
|---|---|
| `applied_state` 命令：读回 Claude 正在用的网关配置 | 概览页逐槽位「已生效 / 未应用」和「有 N 处改动」要知道 Claude 那边实际写着什么。只按哈希只能说「整份脏了」，说不出是哪几行 |
| `models_dev_context` 配置字段 + `fill_known_context` | 模型选择器要标上下文；新挑的模型保存时补上 `context_limit`，1M 开关不用等下次同步才能自判断。老用户没有这份数据时启动补抓一次 |
| `available_models` 返回 `{id, context}` | 同上 |
| `reveal_claude_config` 命令 | 设置页「打开配置目录」：在访达里选中那份配置文件（里面没有 API 密钥） |
| `LogEntry` 加 `id / slot / duration_ms / usage / cost_usd / detail`；`get_log_stats` | 日志页。响应体旁路扫描 usage，流传完（或下游断开）时回填；中途断开的不给用量。「今日」统计在后端累计，不受 100 条上限影响 |
| 连不上上游 / 服务商没填地址也记日志 | 以前这两种 502 不进日志：Claude 报错，日志页一片空白 |
| 身份说明按请求换成真实模型（`identity.rs`，回归套件 `identity-note` 用例） | 实测 Chat 模式系统提示词里没有当前模型 ID，只给全部映射时 k3 把自己猜成了表里第一个 Kimi-k2.6。说明里要求模型别向用户提代号、槽位、路由 |
| 日志标记改成给用户看的话，多条之间用「；」连接 | 标记里自带「 · 」（「已整流 · 思考预算过小」）。错误响应体里「ModelLink 尝试过的自动修复」那段没动 |

**界面上偏离设计稿 / 规格的地方**

- 花费显示美元（`$`），设计稿里的 `¥` 不用：models.dev 的价格就是 USD，换算人民币只能估汇率。
- 「自动同步模型费率」去掉了「手填的费率不会被覆盖」——2.1 已经没有手填入口，这句不成立。
- 「添加」类按钮是虚线（规格原文）；设计稿画成实线是因为它全用 box-shadow 描边，画不出虚线。
- 顶栏「概览」tab 上有改动未应用时亮一个小点：PRODUCT.md 原则二要求状态差在任何页面都看得见，
  而日志页 / 设置页的页头没有状态区。
- 顶栏 [↻] = 先落盘草稿再重读全部状态；[+] = 添加服务商弹窗（与服务商页 tab 条末尾共用）。
- macOS 红绿灯：`trafficLightPosition {x:21, y:34}` 把它垂直居中在 64px 顶栏里（用辅助功能 API 实测
  macOS 26 按钮 16px、间距 23px）。品牌字标因此让到 100px 处，窗口够宽时自然回到内容栏左缘。
- 服务商头像换成单色字形 + 品牌色方块（与设计稿一致）；mimo 的图标是两行字标，缩小后看不清，用字母 m。
- 模型选择器是手写的 Popover 列表，没用现成的命令面板库：`npm install` 时 npmmirror 证书报错，不值得为它动 npm 配置。
- toast 移到右下角，免得盖住顶栏的代理状态和页头的应用按钮。
- 1M 开关：已知装不下时禁用；老配置里已经开着的允许关，关了就不能再开。挑了装不下的模型会顺手关掉 1M。
- 引导页预设格子显示真实域名（两个百炼靠 `coding.` / `token-plan.` 前缀区分）。

**浏览器预览**：`npx vite` 后打开 `http://localhost:1420/`，devPreview 支持
`?empty` `?dirty` `?many`（20 个模型，看超出 8 个上限的提示）`?portdown` `?unpricedcfg`（有模型没费率）`?latestart`。

## 还没做的

- 端口被占的提示是系统原生对话框，样式改不了，只能改文案。

已收尾（2026-09-16）：深色模式其余几页和弹窗逐个核对过；更新弹窗、删除确认本来就套了新 token，用真实鼠标点开看过没问题
（脚本合成的点击会让默认焦点按钮亮出焦点环，那是假象）；空日志页做了正式的空状态；README 截图随 2.2.0 重拍；
DMG 背景换成 Archivo + 2.2 深色 token，出 1x/2x 合成 `background.tiff`（出图步骤写在 `background.src.html` 头部注释里）。
