// 平台判断。Windows 首次使用要在 Claude Desktop 里手动填一次网关地址（ModelLink 写不了那一步），
// 界面上只在 Windows 提。浏览器预览里用 ?windows 模拟。
export function isWindows(): boolean {
  if (import.meta.env.DEV && new URLSearchParams(location.search).has("windows")) return true;
  return /Windows/i.test(navigator.userAgent);
}
