import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  applyToClaude,
  configHash,
  getConfig,
  saveConfig,
  setPort as ipcSetPort,
  type Config,
  type Provider,
} from "@/lib/ipc";
import { PRESETS, SLOT_POOL_VERSION, type Preset } from "@/lib/presets";

// ============================================================
// 全局应用状态：配置草稿 + 自动保存(400ms) + 应用状态机 + 页面导航。
// 状态机四态（design.md §8）：clean / dirty / applying / error。
// dirty 判定 = canonical hash（Rust 单一实现，经 config_hash 命令）≠ last_applied_hash。
// ============================================================

export type Page = "overview" | "providers" | "logs" | "settings";
export type ApplyState = "clean" | "dirty" | "applying" | "error";

type Store = {
  /** 配置草稿；加载完成前为 null（链路板显示 Skeleton）。 */
  draft: Config | null;
  /** 就地修改草稿（内部克隆 + 400ms 防抖落盘）。 */
  updateDraft: (fn: (c: Config) => void) => void;
  applyState: ApplyState;
  applyError: string | null;
  apply: () => void;
  /** apply 成功的自增序号，驱动链路板绿闪。 */
  flashNonce: number;

  page: Page;
  setPage: (p: Page) => void;
  selectedProvider: number;
  setSelectedProvider: (i: number) => void;
  /** 自增序号：跳转服务商页后聚焦 API 密钥输入框（预设引导流）。 */
  focusKeyNonce: number;
  /** 从预设（或自定义）创建服务商并跳转编辑。 */
  addProviderFromPreset: (preset: Preset | "custom") => void;
  /** 链路板行点击 → 服务商页选中。 */
  gotoProvider: (index: number) => void;
  /** 本会话内每个服务商的测试结果（「已连通」标记）。 */
  testedOk: Record<number, boolean>;
  setTestedOk: (index: number, ok: boolean) => void;
  /** 服务商增删后索引失效，整体清空。 */
  resetTested: () => void;
  /** 端口热切换：成功后同步草稿 port + 刷新状态 + dirty 重算。 */
  changePort: (port: number) => Promise<void>;
  /** 从后端重新读配置覆盖草稿（费率同步等「后端专管」字段变化后调用）。 */
  reloadConfig: () => Promise<void>;
  /**
   * dirty 是「升级换了槽位池」造成的，而非用户改了配置（§2.2）。
   * 老用户升级后 Claude Desktop 里写的还是旧槽位，不重新应用就用不上原生推理强度选择器。
   */
  poolUpgrade: boolean;
};

const Ctx = createContext<Store | null>(null);

export function AppStoreProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const configQuery = useQuery({ queryKey: ["config"], queryFn: getConfig });

  const [draft, setDraft] = useState<Config | null>(null);
  const [dirty, setDirty] = useState(false);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [flashNonce, setFlashNonce] = useState(0);

  const [page, setPage] = useState<Page>("overview");
  const [selectedProvider, setSelectedProvider] = useState(0);
  const [focusKeyNonce, setFocusKeyNonce] = useState(0);
  const [testedOk, setTestedOkMap] = useState<Record<number, boolean>>({});

  const draftRef = useRef<Config | null>(null);
  draftRef.current = draft;
  const saveTimer = useRef<number | undefined>(undefined);
  const saveSeq = useRef(0);

  // 首次加载：初始化草稿 + 初始 dirty 判定
  useEffect(() => {
    if (configQuery.data && draftRef.current === null) {
      const cfg = structuredClone(configQuery.data);
      cfg.providers ??= [];
      setDraft(cfg);
      configHash(cfg)
        .then((h) =>
          // 空配置无可应用（apply 会校验失败），不算 dirty
          setDirty(cfg.providers.length > 0 && h !== (cfg.last_applied_hash ?? "")),
        )
        .catch(() => {});
    }
  }, [configQuery.data]);

  /** 立即落盘当前草稿并刷新 dirty（防抖到期 / apply 前 flush 共用）。 */
  const flushSave = useCallback(async () => {
    window.clearTimeout(saveTimer.current);
    const cfg = draftRef.current;
    if (!cfg) return;
    const seq = ++saveSeq.current;
    try {
      await saveConfig(cfg);
      const cur = draftRef.current ?? cfg;
      const h = await configHash(cur);
      if (seq === saveSeq.current) {
        // 空配置无可应用，不算 dirty
        setDirty(cur.providers.length > 0 && h !== (cur.last_applied_hash ?? ""));
      }
    } catch (e) {
      toast.error(`保存失败：${String(e)}`);
    }
  }, []);

  const updateDraft = useCallback(
    (fn: (c: Config) => void) => {
      setDraft((prev) => {
        if (!prev) return prev;
        const next = structuredClone(prev);
        fn(next);
        return next;
      });
      setApplyError(null);
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => void flushSave(), 400);
    },
    [flushSave],
  );

  const apply = useCallback(() => {
    if (applying || !draftRef.current) return;
    setApplying(true);
    setApplyError(null);
    void (async () => {
      try {
        await flushSave();
        await applyToClaude();
        const fresh = await getConfig();
        qc.setQueryData(["config"], fresh);
        setDraft((prev) =>
          prev
            ? {
                ...prev,
                last_applied_hash: fresh.last_applied_hash,
                last_applied_at: fresh.last_applied_at,
              }
            : prev,
        );
        const h = await configHash(draftRef.current!);
        setDirty(h !== (fresh.last_applied_hash ?? ""));
        setFlashNonce((n) => n + 1);
        toast.success("已应用，Claude Desktop 正在重启...");
      } catch (e) {
        const msg = String(e);
        setApplyError(msg);
        toast.error(`应用失败：${msg}`);
      } finally {
        setApplying(false);
      }
    })();
  }, [applying, flushSave, qc]);

  const addProviderFromPreset = useCallback(
    (preset: Preset | "custom") => {
      let newIndex = 0;
      updateDraft((c) => {
        const p: Provider =
          preset === "custom"
            ? { target_url: "", api_key: "", models: [], thinking_effort: "" }
            : {
                target_url: preset.url,
                api_key: "",
                // 不默认开 1M —— 预设里多数模型上下文是 200K/256K（见 ProviderEditor 注释）
                models: preset.models.map((name) => ({ name, to_1m: "" })),
                thinking_effort: "",
              };
        c.providers.push(p);
        newIndex = c.providers.length - 1;
      });
      setSelectedProvider(newIndex);
      setPage("providers");
      setFocusKeyNonce((n) => n + 1);
    },
    [updateDraft],
  );

  const gotoProvider = useCallback((index: number) => {
    setSelectedProvider(index);
    setPage("providers");
  }, []);

  const setTestedOk = useCallback((index: number, ok: boolean) => {
    setTestedOkMap((m) => ({ ...m, [index]: ok }));
  }, []);

  const resetTested = useCallback(() => setTestedOkMap({}), []);

  const changePort = useCallback(
    async (port: number) => {
      const status = await ipcSetPort(port); // 失败抛错，由调用方 toast
      setDraft((prev) => (prev ? { ...prev, port: status.port } : prev));
      await qc.invalidateQueries({ queryKey: ["proxy-status"] });
      await qc.invalidateQueries({ queryKey: ["config"] });
      // 端口参与 canonical hash：切换后触发 dirty 重算（提示重新应用）
      window.setTimeout(() => void flushSave(), 0);
      toast.success(`代理已切换到 127.0.0.1:${status.port}，请重新应用到 Claude Desktop`);
    },
    [flushSave, qc],
  );

  // 应用过（有 hash）但记的是旧池代号 → 这次 dirty 是升级带来的
  const poolUpgrade =
    !!draft?.last_applied_hash && (draft.last_applied_pool ?? "") !== SLOT_POOL_VERSION;

  const reloadConfig = useCallback(async () => {
    const fresh = await getConfig();
    qc.setQueryData(["config"], fresh);
    setDraft(structuredClone(fresh));
    const h = await configHash(fresh);
    setDirty(fresh.providers.length > 0 && h !== (fresh.last_applied_hash ?? ""));
  }, [qc]);

  const applyState: ApplyState = applying
    ? "applying"
    : applyError
      ? "error"
      : dirty
        ? "dirty"
        : "clean";

  return (
    <Ctx.Provider
      value={{
        draft,
        updateDraft,
        applyState,
        applyError,
        apply,
        flashNonce,
        page,
        setPage,
        selectedProvider,
        setSelectedProvider,
        focusKeyNonce,
        addProviderFromPreset,
        gotoProvider,
        testedOk,
        setTestedOk,
        resetTested,
        changePort,
        reloadConfig,
        poolUpgrade,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function useAppStore(): Store {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useAppStore must be used within AppStoreProvider");
  return ctx;
}

export { PRESETS };
