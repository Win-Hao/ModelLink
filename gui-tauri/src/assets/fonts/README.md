# 界面字体

| 文件 | 字体 | 用途 |
|---|---|---|
| `archivo-latin.woff2` | [Archivo](https://github.com/Omnibus-Type/Archivo)（Omnibus-Type） | 拉丁字母与数字 |
| `spline-sans-mono-latin.woff2` | [Spline Sans Mono](https://github.com/SorkinType/SplineSansMono)（SorkinType） | 技术值：槽位名、模型名、URL、端口、计数 |

两者均为 SIL Open Font License 1.1，版权与许可声明保留在字体的 name 表里。
中文不打包，走 `"PingFang SC", "Microsoft YaHei"` 系统栈。

## 怎么生成的

源文件取自 [google/fonts](https://github.com/google/fonts) 的 `ofl/archivo` 与 `ofl/splinesansmono`，
用 fonttools 做两件事：

1. **收窄轴**：Archivo 固定 `wdth=100`，两者 `wght` 都限制在 400–700（界面只用到这段）。
2. **子集**：只留 Google Fonts 的 latin 区段，外加 `U+2190-2193`（箭头；Spline Sans Mono 没有，会回退到系统等宽字体）。
   额外保留 `tnum` / `case` / `zero` 等排版特性。

要补字形（比如某个符号渲染成了回退字体），改区段后照此重新生成，并同步 `index.css` 里 `@font-face` 的 `unicode-range`。
