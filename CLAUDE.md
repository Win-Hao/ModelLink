# ModelLink 开发须知

把国产大模型接入 Claude Desktop 的本地代理。Tauri v2 + React 前端，Rust 后端。

```
gui-tauri/src-tauri/src/
  config.rs      配置模型、槽位分配、费率结构
  proxy.rs       127.0.0.1:5678 转发链（thinking 注入 / 错误整流 / SSE 心跳）
  gateway.rs     写 Claude Desktop 的配置（唯一写入点：write_gateway_keys + apply_to_claude_desktop）
  models_dev.rs  从 models.dev 同步费率与模型清单
  desktop_version.rs  按 Claude Desktop 版本决定写哪些键
gui-tauri/regression/  新旧二进制对照的行为回归（run.sh / drive.sh / upstream.py）
docs/design-2.1-follow-desktop.md  2.1 的设计依据；§七 是实施期间的实测修订，先读它
```

---

## 一、动 Claude Desktop 的配置键之前，先去 app.asar 核实

**这是本项目最重要的一条。** 2.1 那轮凡是去核实的地方，都发现设计文档写错了：

| 文档说 | 实际（app.asar） |
|---|---|
| 费率四个字段全部可选 | **全必填**，且值域 [0, 10000]。缺一个字段那行会被拒，可能让整张表失效 |
| 换 slot 池会改变 config_hash | 不会。hash 只覆盖 providers + port |
| Kimi effort 默认 max | 未设置时走 high |

搜索方法（`strings`/`grep -o` 会因正则复杂度限制失败，别用）：

```bash
A=/Applications/Claude.app/Contents/Resources/app.asar
python3 - "$A" <<'PY'
import sys, re
d = open(sys.argv[1], 'rb').read()
for m in list(re.finditer(rb'yourKeyName', d))[:3]:
    h = m.start()
    print(d[max(0,h-400):h+600].decode('utf-8','replace').replace('\n',' '))
PY
```

要看的东西：zod 校验式（有没有 `.optional()`、`.min()/.max()`、枚举 `Rn(Ba)`）、
`availableInVersion`、`predicates.show`（字段间的依赖）、以及 `description` 里的长文说明
——那段常常写着别处没有的机制细节（例："切换档位会破坏前缀缓存命中"）。

已核实的 schema 速查表在 `docs/design-2.1-follow-desktop.md` §七⑧。

**写坏一个键的代价是整份配置失效**，所以：值域要钳位、枚举要过滤、
`.min(1)` 的字符串键在没内容时必须**删键**而不是写空串。

---

## 二、这个产品的第一原则：不制造静默错误

2.1 整轮都在消灭「用户以为 A、实际是 B 且无从察觉」的情况。已经修掉的：

- 未映射槽位悄悄换成第一个模型 → 改为 400 报错
- 服务商级 effort 覆盖掉桌面端选的档位 → 改为透传优先
- 模型开着 1M 但上游只有 256K → 界面标出真实上限
- 费率缺失时按 Anthropic 官方价估算（假账单）→ 所有模型都有价才开启费用显示

**新功能的评判标准**：如果它可能让用户在不知情的情况下得到与预期不符的结果，
要么别做，要么把真相摊开。「健康检查本地应答」就是因为「上游挂了也显示正常」被整个删掉的。

上游侧同样有静默失败：实测 Kimi 的 `/coding/` 端点对不存在的模型名返回 200 直接给默认模型。
所以**任何「发个请求看状态码」的判断都不足以证明生效**——要证明 effort 真的生效，
唯一办法是比较不同档位下的 `usage.output_tokens_details.thinking_tokens`。

---

## 三、改完之后：diff 实际写入的文件

这轮抓到的 bug 里，**没有一个是测试发现的**，全部来自核实和 diff：

- 身份说明文本混进了 9 个空格（Rust 多行字符串续行）——diff 网关配置时看见的
- 标题生成优化被服务商默认档挡死——通读转发链时发现的顺序错误
- models.dev 返回空表会清空用户已有费率——补回归盲区时发现的
- 点完「应用」仍显示「尚未应用」——追前端 hash 计算时发现的

所以流程是：跑测试 → **应用一次并 diff 网关配置** → 再看结论。

```bash
# 对真实 HOME 跑生产 apply（临时加一个 #[ignore] 测试，跑完 git checkout 掉）
cargo test tmp_apply_for_real -- --ignored --nocapture
diff <(python3 -m json.tool --sort-keys /tmp/before.bak) \
     <(python3 -m json.tool --sort-keys ~/Library/Application\ Support/Claude-3p/configLibrary/a0a0a0a0-*.json)
```

⚠️ `git checkout -- <file>` 会把**同一文件里未提交的其它修改一起还原**。加临时测试前先提交手头的改动。

---

## 四、回归套件

```bash
unzip -o -q ModelLink-macOS.zip -d /tmp/mlv1 && xattr -cr /tmp/mlv1   # 取 v1 二进制
cd gui-tauri
OLD_BIN=/tmp/mlv1/ModelLink.app/Contents/MacOS/modellink \
NEW_BIN="$PWD/src-tauri/target/debug/modellink" bash regression/run.sh
```

原理：同一组请求分别打到 v1 和新版，逐用例比对转发出去的字节。
**表外用例必须逐字节一致；有意改变的行为进 `run.sh` 顶部的 `DIVERGE` 表并断言新行为。**

跑之前：`pkill -f "target/debug/modellink"`、`pkill -f "tauri dev"`，
端口 5678 / 9999 / 9998 必须空闲，且要 `cargo build` 过。

两个容易踩的：

- **用例顺序有意义**。整流成功会往服务商能力缓存里写结论，后续同 provider+model 的用例
  会被预先改写、少一次转发。会写缓存的用例必须排最后，或用 9998 那个独立上游。
- v1 认的是 2.0 的 legacy 槽位名，drive.sh 通过 `SLOT0..SLOT4` 环境变量给两边喂各自的槽位。

---

## 五、models.dev 集成

费率和模型清单都来自 `https://models.dev/api.json`（社区维护，MIT，CI 自动同步各家目录）。

- **一次抓取两用**：`fetch_catalog` 同时返回费率表和模型 ID 索引。
  费率只收有 `cost` 的条目；模型清单要列全，两个独立解析函数。
- **只存认得的 11 家**。api.json 有 213 家，全存进 config.json 会让它从 600 字节涨到 238 KB，
  而那文件每次编辑都要重写。`provider_id_for_url` 的值域必须与 `known_provider_ids()` 一致（有测试钉住）。
- **上下文可以跨服务商借，价格绝对不能**。上下文是模型自身属性；价格是服务商属性，
  同一模型在订阅制那家是 0、在按量付费那家可能是 0.95。
- **查不到就保持原值，绝不清空**。models.dev 返回空表 / 改结构 / 下掉条目都会让查询落空，
  一旦清空费率行，引擎立刻退回按 Anthropic 官方价估算。价格略旧远好过假账单。
- 发版前 `npm run sync-models` 刷新打包进去的兜底快照，改动一起提交。

---

## 六、几个具体的坑

- **能力缓存键要归一化**：`resolve_model` 会给 1M 变体加 `[1m]` 后缀，不 strip 就会
  产生两条互不相通的缓存记录。
- **整流器的判定顺序**：budget 与 thinking 块的报错同样是 400 且请求常带 `output_config`，
  让 effort 的投机分支先命中就会删错字段。明确点名的整流器优先，投机分支排最后。
- **前端 hooks 不能放在 early return 之后**（`if (!draft || !p) return null;`）——tsc 查不出来。
- **后端专管的字段要在 `save_config` 里保住**（`pricing_synced` / `context_limit` /
  `models_dev_models` / `models_dev_context` / `last_applied_*`）。否则前端一份旧草稿保存下去就会抹掉后台同步的结果。
  这类字段进 `canonical_hash`，所以 dirty 判定必须用**后端返回的那份**配置算，用草稿算必然出错。
- **`[1m]` 后缀永远不会到代理**：引擎会剥掉它、改发 `anthropic-beta: context-1m-2025-08-07` 头。
- **本机挂着系统 HTTP 代理时，reqwest 连 127.0.0.1 也会走代理**（macOS 代理例外列表不生效）：
  连不上的端口会变成代理回的 502。起假上游的 Rust 测试一律 `Client::builder().no_proxy()`。
- **Chat 模式的系统提示词里没有当前模型 ID**：只在 `organizationInstructions` 里列全部
  「代号 = 模型」，模型不知道自己是哪一条，映射一多就猜错。代理转发时按请求把那段说明换成
  这一次的真实模型（`identity.rs`），并要求模型别把代号、槽位这些内部细节说给用户听。
- **「已生效」看的是 Claude 那边真正写着的东西**：概览页逐槽位比对 `applied_state`
  （读回网关配置），不是 ModelLink 自己记得写过什么。配置被别的工具改过时照实显示「未应用」。
- **「要不要应用」也不看哈希**（2.2）：槽位比对 + 后端 `pending_apply`（网关地址、费率表逐值比）。
  只改密钥 / 地址 / 默认档不算——代理当场就用上了，别让用户白白重启 Claude。
  但 `set_port` 会**立刻**改写文件里的网关地址，文件对得上不代表 Claude 用上了（要重启才读），
  所以端口单独记 `last_applied_port`（不进 canonical hash，否则老用户升级全变脏）。它为空时前端退回按哈希判断。
- **代理没在跑时，任何地方都不能写「已生效」**：概览页的连通链、页头按钮、设置页排查读的是同一份
  `src/lib/health.ts`，改判定只改这一处。

---

## 七、界面：控件的去留标准

问一句：**用户要基于什么知识、在什么情况下做这个决定？**
答案是「永远不需要动」或「得懂 ModelLink 内部机制才能决定」的，就别做成设置项。

2.1 那轮据此把设置页从 13 个控件砍到 6 个。三种处理：

- **整个删**（功能本身就有问题，如健康检查本地应答）
- **删决策、留行为**（开着永远更好的，如标题生成省思考、SSE 心跳、槽位映射说明）
- **保留**（真需要用户判断的：端口、外观、自启、费率同步开关）

另外：说明文字别写实现细节。「实测花掉 88 个思考 token」是写给开发者的，
用户只需要知道「开着更省钱」。

---

## 八、常用命令

```bash
cd gui-tauri
npm install && npm run tauri dev      # 开发（会写真实的 Claude-3p 配置）
npm run check                          # tsc
npm run sync-models                    # 刷新模型快照（发版前）
cd src-tauri && cargo test             # 单测
cargo test -- --ignored --test-threads=1   # 改 HOME 的串行用例
cargo clippy --all-targets             # 必须零告警
```

发版：版本号四处同步（`package.json` / `tauri.conf.json` / `Cargo.toml` / `Cargo.lock`），
签名公证见 `gui-tauri/SIGNING.md`。
