# 品牌图标

顶栏左上角的 logo，就是应用图标本身，不另画。

| 文件 | 尺寸 | 用途 |
|---|---|---|
| `app-icon.png` | 26 × 26 | 1x 屏 |
| `app-icon@2x.png` | 52 × 52 | Retina / 200% 缩放 |
| `app-icon@3x.png` | 78 × 78 | 150% 以上缩放的 Windows 屏兜底 |

## 怎么生成的

源文件是 `gui-tauri/branding/icon-source.png`（1024 × 1024）。圆角方块外有 102px 透明留白，
在顶栏里缩到 26px 会显得小一圈，所以先裁掉留白再缩：

```bash
SRC=gui-tauri/branding/icon-source.png
OUT=gui-tauri/src/assets/brand
crop() { magick "$SRC" -crop 819x819+102+102 +repage -filter Lanczos -resize "$1x$1" -strip "PNG32:$OUT/$2"; }
crop 26 app-icon.png
crop 52 app-icon@2x.png
crop 78 app-icon@3x.png
```

换了应用图标要照此重新生成。
