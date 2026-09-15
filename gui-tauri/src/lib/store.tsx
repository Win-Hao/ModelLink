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
  pendingApply,
  saveConfig,
  setPort as ipcSetPort,
  testProvider,
  type Config,
  type PendingApply,
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
//
// dirty（要应用）看的是 Claude Desktop 那边真正写着的东西，不是配置哈希：
//   槽位 / 显示名 / 1M 对不上（读回网关配置逐条比）、网关地址或端口对不上、费率表过期。
// 只改了密钥 / 地址 / 默认档时代理当场就用上了，不算 dirty —— 不必为它重启一次 Claude。
// 例外：2.2 之前应用的（后端不知道当时的端口）退回按哈希判断，应用一次之后就按上面的规则。
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
  /** 槽位之外还欠 Claude 的：网关地址、端口、费率表（后端读回判断）；没读到为 null。 */
  pendingApply: PendingApply | null;
  /** 改过、已经保存并且当场生效（密钥 / 地址 / 默认档），不需要应用。 */
  savedLive: boolean;

  page: Page;
  setPage: (p: Page) => void;
  selectedProvider: number;
  setSelectedProvider: (i: number) => void;
  /** 服务商页待聚焦的字段（预设引导流、「去填 API 密钥」）；nonce 保证同一字段能再次触发。 */
  focusRequest: { provider: number; field: "url" | "key"; nonce: number } | null;
  /** 跳到服务商页并聚焦某个字段。 */
  gotoProviderField: (index: number, field: "url" | "key") => void;
  /** 自增序号：设置页收到后滚到并聚焦端口输入框（「换一个端口」）。 */
  focusPortNonce: number;
  gotoPort: () => void;
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
  /** 跳到服务商页并展开某一行的模型选择器（「去选模型」）。 */
  gotoModelPick: (provider: number, row: number) => void;
  /** 链路板行点击 → 服务商页选中。 */
  gotoProvider: (index: number) => void;
  /** 这个服务商（按地址 + 密钥）上次「测试连接」的结论；没测过为 undefined。跨会话保留。 */
  verificationFor: (p: Provider) => Verification | undefined;
  recordVerification: (p: Provider, v: Verification) => void;
  /** 测一遍这些服务商（下标；缺省 = 全部填完整的）。结论写进 verification。 */
  testProviders: (indices?: number[]) => Promise<void>;
  /** 正在测试的服务商（按地址 + 密钥）。 */
  isTesting: (p: Provider) => boolean;
  /** 首次应用成功后的交接卡片：告诉用户去 Claude 里选哪个模型（Windows 还有手动那一步）。 */
  handoff: { model: string } | null;
  showHandoff: () => void;
  dismissHandoff: () => void;
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

  const pendingQuery = useQuery({
    queryKey: ["pending-apply"],
    queryFn: pendingApply,
    refetchOnWindowFocus: true,
  });

  const [page, setPage] = useState<Page>("overview");
  const [selectedProvider, setSelectedProvider] = useState(0);
  const [focusRequest, setFocusRequest] = useState<Store["focusRequest"]>(null);
  const [focusPortNonce, setFocusPortNonce] = useState(0);
  const [testing, setTesting] = useState<ReadonlySet<string>>(new Set());
  const [handoff, setHandoff] = useState<Store["handoff"]>(null);
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
      // 后端按刚保存的配置重新比对 Claude 那边（费率表跟着模型走）
      void qc.invalidateQueries({ queryKey: ["pending-apply"] });
    } catch (e) {
      toast.error(`保存失败：${String(e)}`);
    }
  }, [qc]);

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
    // 第一次接入：成功后回概览页，告诉用户去 Claude 里选哪个模型
    const firstApply = !draftRef.current.last_applied_at;
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
        await qc.invalidateQueries({ queryKey: ["pending-apply"] });
        setFlashNonce((n) => n + 1);
        if (firstApply) {
          // 第一次接入由概览页的交接卡片来说（下一步去 Claude 里选哪个模型），不再弹 toast
          const first = flattenModels(fresh)[0];
          if (first) setHandoff({ model: first.name });
          setPage("overview");
        } else {
          toast.success("已应用，Claude Desktop 正在重启…");
        }
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
      setFocusRequest({ provider: newIndex, field: preset === "custom" ? "url" : "key", nonce: Date.now() });
    },
    [updateDraft],
  );

  const clearModelPickRequest = useCallback(() => setModelPickRequest(null), []);

  const gotoModelPick = useCallback((provider: number, row: number) => {
    setSelectedProvider(provider);
    setPage("providers");
    setModelPickRequest({ provider, row });
  }, []);

  const gotoProvider = useCallback((index: number) => {
    setSelectedProvider(index);
    setPage("providers");
  }, []);

  const gotoProviderField = useCallback((index: number, field: "url" | "key") => {
    setSelectedProvider(index);
    setPage("providers");
    setFocusRequest({ provider: index, field, nonce: Date.now() });
  }, []);

  const gotoPort = useCallback(() => {
    setPage("settings");
    setFocusPortNonce((n) => n + 1);
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

  const testProviders = useCallback(
    async (indices?: number[]) => {
      const providers = draftRef.current?.providers ?? [];
      const targets = (indices ?? providers.map((_, i) => i))
        .map((i) => providers[i])
        .filter((p): p is Provider => !!p && !!p.target_url && !!p.api_key && p.models.some((m) => m.name));
      const keys = targets.map(verificationKey);
      setTesting((cur) => new Set([...cur, ...keys]));
      await Promise.all(
        targets.map(async (p) => {
          // 测第一个有名字的模型：密钥和地址对不对，一个模型就能说明
          const model = p.models.find((m) => m.name)!.name;
          try {
            const r = await testProvider(p.target_url, p.api_key, model);
            recordVerification(p, { ok: r.ok, at: Date.now(), message: r.message });
          } catch (e) {
            recordVerification(p, { ok: false, at: Date.now(), message: String(e) });
          }
        }),
      );
      setTesting((cur) => new Set([...cur].filter((k) => !keys.includes(k))));
    },
    [recordVerification],
  );

  const isTesting = useCallback((p: Provider) => testing.has(verificationKey(p)), [testing]);

  const showHandoff = useCallback(() => {
    const first = draftRef.current ? flattenModels(draftRef.current)[0] : undefined;
    setHandoff({ model: first?.name ?? "" });
    setPage("overview");
  }, []);
  const dismissHandoff = useCallback(() => setHandoff(null), []);

  const changePort = useCallback(
    async (port: number) => {
      const status = await ipcSetPort(port); // 失败抛错，由调用方 toast
      setDraft((prev) => (prev ? { ...prev, port: status.port } : prev));
      await qc.invalidateQueries({ queryKey: ["proxy-status"] });
      await qc.invalidateQueries({ queryKey: ["config"] });
      await qc.invalidateQueries({ queryKey: ["applied-state"] });
      await qc.invalidateQueries({ queryKey: ["pending-apply"] });
      // 端口参与 canonical hash：切换后触发 dirty 重算（提示重新应用）
      window.setTimeout(() => void flushSave(), 0);
      toast.success(`代理已切换到 127.0.0.1:${status.port}，应用一次 Claude 才会连到新端口`);
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
    await qc.invalidateQueries({ queryKey: ["pending-apply"] });
  }, [flushSave, qc]);

  const diff =
    draft && appliedQuery.data ? diffApplied(flattenModels(draft), appliedQuery.data.models) : null;
  const pendingSlots = diff?.pending ?? NO_SLOTS;
  // 空配置无可应用（apply 会校验失败），Claude 里残留的旧条目也不算
  const pendingCount = diff && draft!.providers.length > 0 ? diff.pending.size + diff.removed : 0;

  const pa = pendingQuery.data ?? null;
  const hasProviders = !!draft && draft.providers.length > 0;
  // 后端知道上次应用时的端口 = 2.2 之后应用过，可以完全按 Claude 实际写着的判断；
  // 否则（老版本应用的 / 从没应用过）哈希变了就算要应用，宁可多提示一次
  const byClaude = !!pa && pa.port_changed !== null;
  const otherPending = !!pa && (pa.gateway || pa.pricing || pa.port_changed === true || pa.egress || pa.identity);
  const needsApply = hasProviders && (pendingCount > 0 || otherPending || (!byClaude && dirty));
  const savedLive = hasProviders && dirty && byClaude && !needsApply;

  const applyState: ApplyState = applying
    ? "applying"
    : applyError
      ? "error"
      : needsApply
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
        pendingApply: pa,
        savedLive,
        page,
        setPage,
        selectedProvider,
        setSelectedProvider,
        focusRequest,
        gotoProviderField,
        focusPortNonce,
        gotoPort,
        modelPickRequest,
        clearModelPickRequest,
        gotoModelPick,
        pickerOpen,
        setPickerOpen,
        addProviderFromPreset,
        gotoProvider,
        verificationFor,
        recordVerification,
        testProviders,
        isTesting,
        handoff,
        showHandoff,
        dismissHandoff,
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
