import { useQuery } from "@tanstack/react-query";

import { ApplyButton } from "@/components/ApplyStatus";
import { HandoffCard } from "@/components/HandoffCard";
import { LinkChain } from "@/components/LinkChain";
import { ModelLinkTable } from "@/components/ModelLinkTable";
import { PageHeader } from "@/components/PageHeader";
import { PresetGrid } from "@/components/PresetGrid";
import { SetupSteps } from "@/components/SetupSteps";
import { useHealth } from "@/lib/health";
import { getLogs } from "@/lib/ipc";
import { MAX_MODELS, flattenModels, formatSince } from "@/lib/presets";
import { useAppStore } from "@/lib/store";

/** 概览页（design-2.2.md §6.1）；零服务商时变引导页（§6.5）。 */
export function OverviewPage() {
  const { draft, addProviderFromPreset, setPage } = useAppStore();
  const { links, apply } = useHealth();
  const logsQuery = useQuery({ queryKey: ["logs"], queryFn: getLogs, refetchInterval: 2000 });

  // 空状态：概览页即引导
  if (draft && draft.providers.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto pb-5">
        <PageHeader
          title="选择一个服务商开始"
          sub="配置 API 密钥后，一键接入 Claude Desktop。全程三步，大约两分钟。"
        />
        <SetupSteps
          className="mb-[22px]"
          steps={[
            { title: "挑一个服务商", sub: "下面九格里点一个，地址和推荐模型会自动填好", state: "current" },
            { title: "粘贴 API 密钥", sub: "从服务商后台复制，点「测试连接」验证一下", state: "upcoming" },
            { title: "应用到 Claude Desktop", sub: "Claude 会自动重启，模型选择器里就能看到你的模型", state: "upcoming" },
          ]}
        />
        <PresetGrid onPick={addProviderFromPreset} />
        <div className="mt-3.5 rounded-md px-3.5 py-[11px] text-center text-[12px] text-fg3 inset-ring inset-ring-hair2">
          接入之后，这里会显示 Claude 到你的服务商这一路通不通，以及接进去的每个模型
        </div>
      </div>
    );
  }

  const used = draft ? flattenModels(draft).length : 0;
  const appliedSince = formatSince(draft?.last_applied_at);
  const logs = logsQuery.data ?? [];
  const latest = logs[logs.length - 1];
  const latestFailed = !!latest && (latest.error || latest.status !== 200);

  return (
    <div className="flex min-h-0 flex-1 flex-col pb-5">
      <PageHeader
        title="模型链路"
        sub="这些模型会出现在 Claude Desktop 的模型选择器里。"
        right={<ApplyButton action={apply} />}
      />
      <HandoffCard />
      <LinkChain links={links} />
      <ModelLinkTable />
      <footer className="flex flex-none items-center gap-6 px-0.5 pt-3.5 text-[12.5px] text-fg3">
        <span>
          <b className="mono font-medium text-fg2">
            {used} / {MAX_MODELS}
          </b>{" "}
          个模型 · <b className="mono font-medium text-fg2">{draft?.providers.length ?? 0}</b> 个服务商
        </span>
        <span className="ml-auto">
          {appliedSince ? (
            <>
              上次应用 <b className="font-medium text-fg2">{appliedSince}</b>
            </>
          ) : (
            "还没有应用过"
          )}
          {" · "}
          {latest ? (
            <button
              onClick={() => setPage("logs")}
              className="rounded-[4px] transition-colors outline-none hover:text-fg2 focus-visible:ring-[3px] focus-visible:ring-ring/40"
              title="查看请求日志"
            >
              最近请求 <b className="mono font-medium text-fg2">{latest.time}</b>
              {latestFailed && <span className="ml-1.5 text-danger">失败</span>}
            </button>
          ) : (
            "还没有请求"
          )}
        </span>
      </footer>
    </div>
  );
}
