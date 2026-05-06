# ModelLink

> **本软件完全免费，仅供个人学习和非商业用途。严禁任何形式的商业化行为，包括但不限于出售、收费分发、嵌入付费产品等。**
>
> 作者：**抖音Winhao学AI**（抖音号：**54927876676**）
>
> 如果你是花钱买到的这个软件，你被骗了，请举报卖家。

让 Claude Desktop 桌面端接入任意第三方 API 模型的本地代理工具。

## 功能

- 将第三方模型（DeepSeek、Kimi、智谱 GLM 等）接入 Claude Desktop
- 支持同时配置多个 API 服务商
- 支持 1M 上下文模型变体
- 可视化配置界面，无需手动编辑配置文件
- 菜单栏/系统托盘常驻，关闭窗口后代理继续运行
- 深色/亮色/跟随系统主题切换
- 连接测试、请求日志、开机自启

## 下载

从 [Releases](../../releases) 页面下载：

| 平台 | 文件 |
|------|------|
| macOS | `ModelLink.dmg` |
| Windows | `ModelLink-Windows.zip` |

## 安装

### macOS

1. 下载 `ModelLink.dmg`，双击打开
2. 将 `ModelLink.app` 拖入「应用程序」文件夹
3. 首次打开如果提示"已损坏"，在终端执行：
   ```bash
   xattr -cr /Applications/ModelLink.app
   ```
4. 双击打开即可

### Windows

1. 解压 `ModelLink-Windows.zip`
2. 确保 `ModelLink.exe` 和 `WebView2Loader.dll` 在同一目录
3. 双击 `ModelLink.exe` 运行
4. 首次运行如果触发 Windows Defender 警告，选择「仍然运行」

## 首次使用

### macOS 首次配置

#### 第一步：初始化 Claude Desktop（仅首次需要）

1. 打开 Claude Desktop，完成初始启动
2. 菜单栏进入 **Help > Troubleshooting > Enable Developer Mode**
3. 重启 Claude Desktop
4. 菜单栏进入 **Developer > Configure third-party inference**
5. Backend 选择 **Gateway (Anthropic-compatible)**
6. Gateway URL 填写 `http://127.0.0.1:5678`
7. API Key 填写 `proxy`
8. 点击 **Apply locally**

完成后，后续所有配置通过 ModelLink 管理，无需再手动操作。

### Windows 首次配置

Windows 上 Claude Desktop 的菜单位置与 macOS 不同，请按以下步骤操作：

#### 第一步：初始化 Claude Desktop（仅首次需要）

1. 打开 Claude Desktop，完成初始启动
2. 点击左上角 **☰ 汉堡菜单**（三条横线图标）
3. 进入 **Help > Troubleshooting > Enable Developer Mode**
4. 完全关闭并重新打开 Claude Desktop
5. 再次点击 **☰ 汉堡菜单**，进入 **Developer > Configure third-party inference**
6. 在弹出的配置面板中：
   - **Backend** 选择 `Gateway (Anthropic-compatible)`
   - **Gateway URL** 填写 `http://127.0.0.1:5678`
   - **API Key** 填写 `proxy`
7. 点击 **Apply locally** 保存

> **说明：** 这一步的作用是把 Claude Desktop 的 API 请求从 Anthropic 官方服务器改为指向本地的 ModelLink 代理（`127.0.0.1:5678`）。之后 ModelLink 会将请求转发到你配置的第三方 API（如 DeepSeek、Kimi 等）。

#### 第二步：配置 ModelLink

1. 双击运行 `ModelLink.exe`（需和 `WebView2Loader.dll` 在同一目录）
2. 点击「添加服务商」
3. 填写你的第三方 API 信息：
   - **API 地址**：填写第三方服务商提供的 API 地址（如 `https://api.deepseek.com/anthropic`）
   - **API 密钥**：填写你在第三方平台申请的 API Key
4. 点击「添加模型」，填写模型名称（填什么名称，Claude Desktop 中就显示什么）
5. 点击「测试」验证连接是否正常
6. 点击「保存配置」
7. 点击「应用到 Claude Desktop」— Claude Desktop 会自动重启

#### 第三步：开始使用

在 Claude Desktop 的模型选择器中选择你配置的模型即可。

> **提示：** ModelLink 需要保持运行。关闭窗口后 ModelLink 会缩小到系统托盘继续工作，不会影响 Claude Desktop 使用。

## 多服务商支持

支持同时配置多个 API 服务商，在 Claude Desktop 中统一管理和切换。

## Windows 注意事项

- 需要 WebView2 Runtime（Windows 10/11 自带 Edge 的通常已有）
- `ModelLink.exe` 和 `WebView2Loader.dll` 需放在同一目录
- 首次运行可能触发 Windows Defender 警告，选择「仍然运行」即可

## 构建

需要 Rust 工具链：

```bash
# macOS
cargo build --release

# Windows（交叉编译）
rustup target add x86_64-pc-windows-gnu
brew install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

## 免责声明与版权

- 本软件由 **抖音Winhao学AI**（抖音号：**54927876676**）开发并免费提供
- **完全免费，不可商业化** — 禁止任何形式的售卖、收费分发、打包进付费产品
- 欢迎免费转发分享，但必须保留原始作者信息，不得篡改或移除软件内的水印和版权声明
- 如发现有人售卖本软件，请联系作者举报

## 许可证

本项目采用 [CC BY-NC-ND 4.0](https://creativecommons.org/licenses/by-nc-nd/4.0/) 许可证 — 署名-非商业性使用-禁止演绎。

- **署名**：必须注明原作者（抖音Winhao学AI）
- **非商业性使用**：不得用于任何商业目的
- **禁止演绎**：不得修改后再分发（防止去除水印后转卖）
