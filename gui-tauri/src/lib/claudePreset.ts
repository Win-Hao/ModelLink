// 「一键使用 Winhao 的配置」里每个开关给用户看的说法。
// 键名与取值以后端 gateway.rs 的 WINHAO_PRESET 为准（对照 app.asar 核实过）；这里只管怎么说。
// 说明写的是「对你有什么影响」，不写实现细节。

export const PRESET_LABELS: Record<string, { name: string; hint?: string }> = {
  coworkTabEnabled: { name: "Cowork 标签页", hint: "长任务：研究、分析、写文档" },
  isClaudeCodeForDesktopEnabled: { name: "Code 标签页", hint: "写代码、跑代码" },
  chatTabEnabled: { name: "Chat 标签页", hint: "日常问答和写作" },
  blockReadsOutsideWorkingDirectories: {
    name: "禁止读取工作目录以外的文件",
    hint: "开着更安全，但 Code 要读别处的文件时会被挡住",
  },
  autoModeEnabled: { name: "Auto 模式", hint: "Code 和 Cowork 里只对有风险的操作弹确认" },
  disableBypassPermissionsMode: { name: "禁用「跳过权限确认」模式", hint: "关着就还能选这个模式" },
  disableBundledSkills: { name: "禁用 Claude Code 自带的技能", hint: "比如深度研究，关着才能用" },
  skillCreationEnabled: { name: "允许自己创建技能" },
  userPluginMarketplacesEnabled: { name: "允许自己添加插件市场" },
  userPluginUploadsEnabled: { name: "允许自己安装插件" },
  disableDeploymentModeChooser: {
    name: "隐藏 Claude.ai 登录",
    hint: "登录页只显示第三方服务，免得登进官方账号绕开 ModelLink",
  },
  disableDeepLinkRegistration: {
    name: "禁止 claude:// 链接打开 Claude",
    hint: "关着时，网页上的链接可以直接打开 Claude（比如添加插件市场）",
  },
  skipWebFetchPreflight: {
    name: "跳过网页抓取的域名检查",
    hint: "Code 抓网页前要先连 api.anthropic.com 核对域名，连不上就每次都失败",
  },
  toolSearchEnabled: {
    name: "按需加载 MCP 工具",
    hint: "开着会给请求加一种新格式，模型服务商那边不认就会直接报错",
  },
  chatAdvancedFileAnalysisEnabled: { name: "高级文件分析", hint: "Chat 里能分析 Excel、PPT 这类附件" },
};

export const onOff = (v: boolean) => (v ? "开" : "关");
