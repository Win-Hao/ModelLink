import { useQuery } from "@tanstack/react-query";

import { ApplyStatusArea } from "@/components/ApplyStatus";
import { ModelLinkTable } from "@/components/ModelLinkTable";
import { PageHeader } from "@/components/PageHeader";
import { PresetGrid } from "@/components/PresetGrid";
import { getLogs } from "@/lib/ipc";
import { MAX_MODELS, flattenModels, formatSince } from "@/lib/presets";
import { useAppStore } from "@/lib/store";

// 首次启动的三步说明（§6.5）：用户是跟着视频走的，先告诉他一共几步、大概多久
const STEPS = [
  { title: "挑一个服务商", sub: "下面九格里点一个，地址和推荐模型会自动填好" },
  { title: "粘贴 API 密钥", sub: "从服务商后台复制，点「测试连接」验证一下" },
  { title: "应用到 Claude Desktop", sub: "Claude 会自动重启，模型选择器里就能看到你的模型" },
];

/** 概览页（design-2.2.md §6.1）；零服务商时变引导页（§6.5）。 */
export function OverviewPage() {
  const { draft, addProviderFromPreset, setPage } = useAppStore();
  const logsQuery = useQuery({ queryKey: ["logs"], queryFn: getLogs, refetchInterval: 2000 });

  // 空状态：概览页即引导
  if (draft && draft.providers.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto pb-5">
        <PageHeader
          title="选择一个服务商开始"
          sub="配置 API 密钥后，一键接入 Claude Desktop。全程三步，大约两分钟。"
        />
        <ol className="mb-[22px] flex flex-none">
          {STEPS.map((s, i) => (
            <li key={s.title} className="flex flex-1 items-start gap-[11px] pr-[22px]">
              <span className="mono flex size-[22px] flex-none items-center justify-center rounded-full bg-accent-weak text-[11.5px] font-bold text-accent">
                {i + 1}
              </span>
              <span>
                <span className="block text-[13px] font-semibold tracking-[-0.01em]">{s.title}</span>
                <span className="mt-[3px] block text-[11.5px] leading-[1.5] text-pretty text-fg3">{s.sub}</span>
              </span>
            </li>
          ))}
        </ol>
        <PresetGrid onPick={addProviderFromPreset} />
        <div className="mt-3.5 rounded-md px-3.5 py-[11px] text-center text-[12px] text-fg3 inset-ring inset-ring-hair2">
          接入之后，这里会变成「模型链路」—— 一眼看清 Claude 里的哪个名字对应你的哪个模型
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
        sub="Claude Desktop 里的每个模型名，实际接到了你的哪个模型。"
        right={<ApplyStatusArea />}
      />
      <ModelLinkTable />
      <footer className="flex flex-none items-center gap-6 px-0.5 pt-3.5 text-[12.5px] text-fg3">
        <span>
          <b className="mono font-medium text-fg2">
            {used} / {MAX_MODELS}
          </b>{" "}
          个槽位已用 · <b className="mono font-medium text-fg2">{draft?.providers.length ?? 0}</b>{" "}
          个服务商
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
              className="transition-colors hover:text-fg2"
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
