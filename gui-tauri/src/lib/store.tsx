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
  appliedState,
  configHash,
  getConfig,
  saveConfig,
  setPort as ipcSetPort,
  type Config,
  type Provider,
} from "@/lib/ipc";
import {
  MAX_MODELS,
  PRESETS,
  SLOT_POOL_VERSION,
  applyErrorText,
  diffApplied,
  flattenModels,
  matchPreset,
  totalModelsRaw,
  type Preset,
} from "@/lib/presets";
import {
  loadVerifications,
  saveVerifications,
  verificationKey,
  type Verification,
} from "@/lib/verification";

// ============================================================
// 全局应用状态：配置草稿 + 自动保存(400ms) + 应用状态机 + 页面导航。
// 状态机四态（design-2.2.md §7）：clean / dirty / applying / error。
// dirty = 配置改过（canonical hash ≠ last_applied_hash，Rust 单一实现）
//       或 Claude Desktop 实际写着的槽位映射和这里对不上（读回网关配置逐条比）。
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
  /** 配置在上次应用之后改过（哈希比对）。 */
  configChanged: boolean;
  /** 已经读回 Claude Desktop 的网关配置 —— 读回之前不对任何槽位下「已生效」的结论。 */
  appliedKnown: boolean;
  /** 和 Claude Desktop 实际写着的对不上的槽位。 */
  pendingSlots: ReadonlySet<string>;
  /** 对不上的条数：上面那些槽位 + Claude 里还留着、这里已经删掉的。 */
  pendingCount: number;

  page: Page;
  setPage: (p: Page) => void;
  selectedProvider: number;
  setSelectedProvider: (i: number) => void;
  /** 自增序号：跳转服务商页后聚焦 API 密钥输入框（预设引导流）。 */
  focusKeyNonce: number;
  /** 「添加服务商」预设网格弹窗（顶栏 [+] 与服务商页共用）。 */
  pickerOpen: boolean;
  setPickerOpen: (open: boolean) => void;
  /**
   * 从预设（或自定义）创建服务商并跳转编辑。
   * 这家预设已经配过时不再新建一个重复的，而是跳过去、打开一行新模型的选择器。
   */
  addProviderFromPreset: (preset: Preset | "custom") => void;
  /** 服务商页待打开的模型选择器（哪家、第几行）；编辑器处理完调 `clearModelPickRequest`。 */
  modelPickRequest: { provider: number; row: number } | null;
  clearModelPickRequest: () => void;
  /** 链路板行点击 → 服务商页选中。 */
  gotoProvider: (index: number) => void;
  /** 这个服务商（按地址 + 密钥）上次「测试连接」的结论；没测过为 undefined。跨会话保留。 */
  verificationFor: (p: Provider) => Verification | undefined;
  recordVerification: (p: Provider, v: Verification) => void;
  /** 端口热切换：成功后同步草稿 port + 刷新状态 + dirty 重算。 */
  changePort: (port: number) => Promise<void>;
  /** 先落盘未保存的编辑，再从后端重新读配置覆盖草稿（费率同步等「后端专管」字段变化后调用）。 */
  reloadConfig: () => Promise<void>;
  /**
   * dirty 是「升级换了槽位池」造成的，而非用户改了配置（§2.2）。
   * 老用户升级后 Claude Desktop 里写的还是旧槽位，不重新应用就用不上原生推理强度选择器。
   */
  poolUpgrade: boolean;
};

const Ctx = createContext<Store | null>(null);
const NO_SLOTS: ReadonlySet<string> = new Set();

export function AppStoreProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const configQuery = useQuery({ queryKey: ["config"], queryFn: getConfig });

  const [draft, setDraft] = useState<Config | null>(null);
  const [dirty, setDirty] = useState(false);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [flashNonce, setFlashNonce] = useState(0);
  // Claude 那边的配置可能在 ModelLink 之外被改（切回窗口时重读一次）
  const appliedQuery = useQuery({
    queryKey: ["applied-state"],
    queryFn: appliedState,
    refetchOnWindowFocus: true,
  });

  const [page, setPage] = useState<Page>("overview");
  const [selectedProvider, setSelectedProvider] = useState(0);
  const [focusKeyNonce, setFocusKeyNonce] = useState(0);
  const [modelPickRequest, setModelPickRequest] = useState<{ provider: number; row: number } | null>(
    null,
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const [verifications, setVerifications] = useState(loadVerifications);

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
      // 用后端返回的那份算 dirty：草稿里可能缺「后端专管」字段（后台同步来的费率、
      // 上下文上限），拿草稿算出来的哈希是错的
      const saved = await saveConfig(cfg);
      const h = await configHash(saved);
      if (seq === saveSeq.current) {
        // 空配置无可应用，不算 dirty
        setDirty(saved.providers.length > 0 && h !== (saved.last_applied_hash ?? ""));
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
        // 整份换掉而不是只挑两个字段：flushSave 已经把用户的编辑落盘了，fresh 里都有；
        // 而草稿可能缺后台同步来的字段，留着它算 dirty 会算错
        setDraft(structuredClone(fresh));
        const h = await configHash(fresh);
        setDirty(h !== (fresh.last_applied_hash ?? ""));
        await qc.invalidateQueries({ queryKey: ["applied-state"] });
        setFlashNonce((n) => n + 1);
        toast.success("已应用，Claude Desktop 正在重启...");
      } catch (e) {
        const msg = applyErrorText(String(e), draftRef.current);
        setApplyError(msg);
        toast.error(`应用失败：${msg}`);
      } finally {
        setApplying(false);
      }
    })();
  }, [applying, flushSave, qc]);

  const addProviderFromPreset = useCallback(
    (preset: Preset | "custom") => {
      const cur = draftRef.current;
      if (!cur) return;

      // 同一家已经配过：再建一个只会多出一份重复的地址和密钥，跳过去加模型才是用户要的
      const existing =
        preset === "custom"
          ? -1
          : cur.providers.findIndex((p) => matchPreset(p.target_url)?.id === preset.id);
      if (existing >= 0) {
        const models = cur.providers[existing].models;
        const blank = models.findIndex((m) => !m.name);
        let row = blank;
        if (blank < 0) {
          if (totalModelsRaw(cur) >= MAX_MODELS) {
            toast.info(`所有服务商的模型加起来最多 ${MAX_MODELS} 个，已经满了`);
          } else {
            row = models.length;
            updateDraft((c) => {
              c.providers[existing].models.push({ name: "", to_1m: "" });
            });
          }
        }
        setSelectedProvider(existing);
        setPage("providers");
        if (row >= 0) setModelPickRequest({ provider: existing, row });
        return;
      }

      // 下标按当前草稿算好再更新 —— 在 updateDraft 的回调里赋值，React 不一定同步执行它
      const newIndex = cur.providers.length;
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
      });
      setSelectedProvider(newIndex);
      setPage("providers");
      setFocusKeyNonce((n) => n + 1);
    },
    [updateDraft],
  );

  const clearModelPickRequest = useCallback(() => setModelPickRequest(null), []);

  const gotoProvider = useCallback((index: number) => {
    setSelectedProvider(index);
    setPage("providers");
  }, []);

  const verificationFor = useCallback(
    (p: Provider) => verifications[verificationKey(p)],
    [verifications],
  );

  const recordVerification = useCallback((p: Provider, v: Verification) => {
    setVerifications((all) => {
      const next = { ...all, [verificationKey(p)]: v };
      saveVerifications(next, draftRef.current?.providers ?? []);
      return next;
    });
  }, []);

  const changePort = useCallback(
    async (port: number) => {
      const status = await ipcSetPort(port); // 失败抛错，由调用方 toast
      setDraft((prev) => (prev ? { ...prev, port: status.port } : prev));
      await qc.invalidateQueries({ queryKey: ["proxy-status"] });
      await qc.invalidateQueries({ queryKey: ["config"] });
      await qc.invalidateQueries({ queryKey: ["applied-state"] });
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
    // 防抖中的编辑先写下去，否则这次读回来的旧配置会把它盖掉
    await flushSave();
    const fresh = await getConfig();
    qc.setQueryData(["config"], fresh);
    setDraft(structuredClone(fresh));
    const h = await configHash(fresh);
    setDirty(fresh.providers.length > 0 && h !== (fresh.last_applied_hash ?? ""));
  }, [flushSave, qc]);

  const diff =
    draft && appliedQuery.data ? diffApplied(flattenModels(draft), appliedQuery.data.models) : null;
  const pendingSlots = diff?.pending ?? NO_SLOTS;
  // 空配置无可应用（apply 会校验失败），Claude 里残留的旧条目也不算
  const pendingCount = diff && draft!.providers.length > 0 ? diff.pending.size + diff.removed : 0;

  const applyState: ApplyState = applying
    ? "applying"
    : applyError
      ? "error"
      : dirty || pendingCount > 0
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
        configChanged: dirty,
        appliedKnown: !!appliedQuery.data,
        pendingSlots,
        pendingCount,
        page,
        setPage,
        selectedProvider,
        setSelectedProvider,
        focusKeyNonce,
        modelPickRequest,
        clearModelPickRequest,
        pickerOpen,
        setPickerOpen,
        addProviderFromPreset,
        gotoProvider,
        verificationFor,
        recordVerification,
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
