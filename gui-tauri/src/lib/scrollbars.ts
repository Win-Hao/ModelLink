/**
 * 滚动条只在滚动时出现，停下一会儿就隐藏 —— 和 macOS 的浮动滚动条一样，平时不占视线。
 *
 * 自定义了 `::-webkit-scrollbar` 之后 WebKit 不再用系统的浮动滚动条，只能自己做：
 * 正在滚动的元素挂上 `data-scrolling`，样式见 index.css。用属性不用 class，免得和 React 管的 className 打架。
 */
export function autoHideScrollbars() {
  const timers = new WeakMap<Element, number>();
  document.addEventListener(
    "scroll",
    (e) => {
      const el = e.target instanceof Element ? e.target : document.scrollingElement;
      if (!el) return;
      el.setAttribute("data-scrolling", "");
      window.clearTimeout(timers.get(el));
      timers.set(
        el,
        window.setTimeout(() => el.removeAttribute("data-scrolling"), 800),
      );
    },
    { capture: true, passive: true },
  );
}
