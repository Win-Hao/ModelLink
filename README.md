<div align="center">

# ModelLink

**让 Claude Desktop 桌面端接入任意第三方 API 模型的本地代理工具**

Kimi · MiniMax · 百炼 · 智谱 GLM · DeepSeek · mimo — 一键切换，无缝使用

[![License: CC BY-NC-ND 4.0](https://img.shields.io/badge/License-CC%20BY--NC--ND%204.0-lightgrey.svg)](https://creativecommons.org/licenses/by-nc-nd/4.0/)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Windows-blue.svg)](#下载)
[![Free](https://img.shields.io/badge/价格-完全免费-brightgreen.svg)](#免责声明与版权)

<br/>

<img src="docs/images/overview.png" alt="ModelLink 概览页 — 模型链路板" width="700"/>

<br/>

</div>

> **本软件完全免费，仅供个人学习和非商业用途。严禁任何形式的商业化行为，包括但不限于出售、收费分发、嵌入付费产品等。**
>
> 作者：**Winhao学AI**（抖音搜索同名，抖音号：**54927876676**）
>
> 如果你是花钱买到的这个软件，你被骗了，请举报卖家。

## 2.1 有什么新东西

- **Claude 里能逐次调推理强度了**：模型选择器直接出现 5 档（low / medium / high / Extra / max）+ auto 思考模式。以前只能在 ModelLink 里按服务商设一次，在 Claude 里选了不生效
- **费用不再是假账单**：新增真实费率表，Usage 页按你实际用的服务商计价，不再按 Anthropic 官方价估算。费率从 [models.dev](https://models.dev) 自动同步，也可手填覆盖；订阅制方案正确显示为 0
- **不再有「悄悄换了个模型」**：请求了没配置的槽位直接报错，不再静默改用第一个模型
- **1M 上下文按真实能力判定**：模型其实装不下 1M 时界面直接标出真实上限，不再默认开
- **长生成不断流**：上游思考期间由 ModelLink 往 Claude 发保活心跳
- **上游报错自动修复**：不认推理强度、thinking 预算过小、验不了历史思考块签名时自动改写重试，你完全无感
- **深度探测**：一次问清一家服务商支持到什么程度（推理强度档位、会不会静默回落模型、缓存透传、1M 支持）
- Chat 标签页默认开启；模型数上限 8 → 20；组织级自定义指令；Claude 出网代理设置

<details>
<summary>2.0 的改动</summary>

2.0 是一次彻底重写（Tauri v2 + React），老用户**无感升级**——端口、配置文件、Claude 接入方式完全不变：

- **模型链路板**：Claude 槽位 → 真实模型的映射直接画出来
- **Claude 里直接显示厂商模型名**：模型选择器显示 `Kimi-k2.6` 这样的真名
- **自动保存 + 应用状态提醒**：改没改、生效没生效，页头一眼看清
- **应用内自动更新**：从 2.0 起新版本自动提醒、一键安装（macOS）
- **代理端口可配置**：默认 5678，被占用时可在设置页更换
- **整页请求日志**：保留最近 100 条，自动刷新
- 修复 Windows 开机自启无效的问题

</details>

<div align="center">
<img src="docs/images/guide.png" alt="首次启动 - 预设网格引导" width="380"/>
&nbsp;&nbsp;&nbsp;&nbsp;
<img src="docs/images/providers.png" alt="服务商页 - 双栏编辑器" width="380"/>

<sub>左：首启引导（内置 8 家服务商预设） &nbsp;|&nbsp; 右：服务商双栏编辑器</sub>
</div>

## 功能

- 将第三方模型（DeepSeek、Kimi、智谱 GLM、MiniMax、百炼、mimo 等）接入 Claude Desktop
- 支持同时配置多个 API 服务商（模型总数最多 20 个），内置主流服务商预设（带品牌图标），也可自定义
- Claude 原生推理强度选择器；真实费率表；1M 上下文变体（按模型真实能力判定）
- 上游报错自动修复重试；长生成保活心跳；连接测试与深度能力探测
- 请求日志、开机自启、深色/亮色/跟随系统主题、代理端口可配置
- 菜单栏/系统托盘常驻，关闭窗口后代理继续运行

> **联网说明**：为了让费用估算保持准确，ModelLink 启动时会向 [models.dev](https://models.dev)
> （社区维护的开源模型数据库）请求一次费率数据，最多 6 小时一次。可在设置页关闭。
> 除此之外和检查软件更新，ModelLink 不会主动联网。

## 下载

从 [Releases](../../releases) 页面下载：

| 平台 | 文件 |
|------|------|
| macOS (Apple Silicon) | `ModelLink_x.y.z_aarch64.dmg` |
| Windows | `ModelLink_x.y.z_x64-setup.exe` |

## 安装

### macOS

1. 下载 `.dmg`，双击打开，把 `ModelLink.app` 拖入「应用程序」
2. 2.0 起为正式签名 + 公证版本，直接双击打开即可

### Windows

1. 下载 `-setup.exe`，双击安装
2. 首次运行如果触发 Windows Defender 警告，选择「仍然运行」

## 首次使用

1. 打开 **ModelLink**，首页会列出内置服务商预设，点一个（或「自定义」）
2. 填写 API 密钥，点「测试连接」验证
3. 点「**应用到 Claude Desktop**」——Claude 会自动重启并接入
4. 在 Claude Desktop 的模型选择器中选择你配置的模型即可

> ModelLink 启动时会自动写入 Claude Desktop 的第三方推理配置；编辑自动保存，无需手动点保存。

### Windows 首次额外一步（仅一次）

ModelLink 会自动写入大部分配置，但首次使用需在 Claude Desktop 中手动完成一步：

1. 打开 Claude Desktop，点左上角 **☰** → **Developer** → **Configure third-party inference**
2. 切换到 **Form view**（左下角）
3. **Gateway URL** 填 `http://127.0.0.1:5678`，**API Key** 填 `proxy`
4. 点 **Apply locally**

> 之后所有模型/服务商的增删改都只在 ModelLink 里完成。

## 从 2.0 升级

直接覆盖安装或用应用内自动更新。2.1 换了 Claude 侧的模型槽位，所以：

- 升级后首次打开会提示「新版为你的模型启用了 Claude Desktop 原生推理强度选择器」
- 点一次「**应用到 Claude Desktop**」即可生效，配置文件无需迁移

## 从 1.x 升级

直接用新安装包覆盖安装即可：

- 配置文件（`~/.claude-model-proxy/config.json`）原样沿用，服务商列表无损保留
- 代理端口（5678）与 Claude 接入方式不变，Claude Desktop 无需重新配置
- 老版本的「开机自启」会自动迁移到新机制
- 从 2.0 起支持应用内自动更新，以后不用再手动下载

## 构建

需要 Node.js ≥ 20 与 Rust 工具链：

```bash
cd gui-tauri
npm install
npm run tauri dev     # 开发
npm run tauri build   # 打包
```

发版流程（签名/公证/latest.json）见 [`gui-tauri/SIGNING.md`](gui-tauri/SIGNING.md)。
1.x 的单文件实现保留在 [`claude-model-proxy/`](claude-model-proxy/)（只读存档）。

## 免责声明与版权

- 本软件按「现状」提供，不对任何使用后果负责；API 费用由所选服务商收取，与本软件无关
- 版权所有 © Winhao学AI，采用 [CC BY-NC-ND 4.0](https://creativecommons.org/licenses/by-nc-nd/4.0/) 许可
- **完全免费，不可商业化**；二次分发请保留出处
