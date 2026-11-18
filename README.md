# Battle Instinct（战斗本能）（双键搓招版）

![演示](demo.gif)

**简体中文** ｜ [English](README.en.md)

基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改的只狼武技 / 忍具 MOD。原版用**方向序列 + 防御/攻击**切换武技；本分支在保留原作者换槽、注入等核心机制的前提下，**重新设计了武技搓招与忍具触发逻辑**（借助 AI 辅助实现）：武技为**两个键位**短窗口连招；忍具为 **首键(方向/攻防/交互/钩锁) + q/t**，或单独 `t` / `q`，直接装备并注入使用。

## 功能范围

当前版本只保留以下三组功能，相关实现与配置已按功能收敛：

1. **武技搓招**：双键短窗口触发武技，含排队与瞬发逻辑。
2. **纸人 + 血纸人**：DLL 实时修正纸人上限；一键脚本写入纸人漂流（血纸人/临时纸人）等 param 数值。
3. **心流**：完美弹反叠层，按层数减免/回复架势。

## 直接使用（推荐）

发行包已包含本 MOD、ModEngine 和纸人漂流所需的 param 文件。将压缩包**保持目录结构**解压到 `sekiro.exe` 所在目录：

```text
Sekiro\
  dinput8.dll
  battle_instinct.cfg
  纸人挂.bat
  dinput8_modengine.dll
  modengine.ini
  mods\param\gameparam\gameparam.parambnd.dcx
```

首次安装或修改 `battle_instinct.cfg` 后：

1. 双击 `纸人挂.bat`，等待脚本提示完成。
2. 完整退出游戏，再重新启动。
3. 游戏内纸人上限由 DLL 每帧实时修正；纸人漂流的临时纸人上限与单次数量由 `纸人挂.bat` 写入的 param 生效。

> `mods\param\gameparam\gameparam.parambnd.dcx` 的目录结构不能改；这是 ModEngine 的 loose param 覆盖文件，也是纸人漂流生效的前提。没有 ModEngine 时，DLL 的纸人上限实时修正仍生效，但纸人漂流等 param 数值保持原版。

### 中文发行包

- [dist/zh/dinput8.dll](dist/zh/dinput8.dll)
- [dist/zh/battle_instinct.cfg](dist/zh/battle_instinct.cfg)
- [dist/zh/纸人挂.bat](dist/zh/纸人挂.bat)　- 纸人挂入口（双击）
- [dist/zh/dinput8_modengine.dll](dist/zh/dinput8_modengine.dll)
- [dist/zh/modengine.ini](dist/zh/modengine.ini)
- [dist/zh/mods/param/gameparam/gameparam.parambnd.dcx](dist/zh/mods/param/gameparam/gameparam.parambnd.dcx)

## 与原版的关系

- **上游**：[@dec32](https://github.com/dec32) 的 [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **本仓库**：理念不同的独立维护版（双键搓招 + 忍具 q/t），**不是**上游官方分支

若你更需要原版手感，请直接使用上游发布页。

## 武技搓招（两个键）

键位含义跟游戏动作绑定，改键后仍有效。双键命中后：换武技槽 → 短暂压制攻击 → 注入防御+攻击放招；第二键按住可继续注入防御+攻击（方便蓄力），松开即停。当前招持续注入时再搓出的招会排队，**末键松手后再等一段 cfg 可调时间**（`# 武技排队等待`，默认约 0.3s；`0s` 则立刻放）才放出。默认检测窗口约 **0.3s**；**首键为 `l`**（如 `l↑`）时约 **0.7s**；**`rl` 樱舞** 单独受 `# rl触发时限` 约束（默认 **0.3s**）：按 `r` 后须在此时间内按 `l`，且触发时须同时按住 `r` 与 `l`。武技侧不建议以 `r`/`l` 为首键：防御优先级较高，攻击键容易按出突刺。源码：`res/battle_instinct_zh.cfg`（中文）、`res/battle_instinct.cfg`（英文）；发行包：`dist/zh/`、`dist/en/`。

| 符号 | 含义 |
| --- | --- |
| `r`（鼠标右键） | 防御 / 格挡 |
| `l`（鼠标左键） | 攻击 |
| `e` | 动作、(长按)吸引 |
| `f` | 钩锁 |
| `↑↓←→` | 移动（WASD / 摇杆） |
| `ee` | 连按两次动作键 |
| `rl` / `re` / `er` / `el` / `rf` / `fr` 等 | 两个键的顺序组合；同一行可用 `/` 写多种搓法，如 `↓l/l↓` |

`e`=交互、`f`=钩锁，均跟随游戏内动作位（改键/手柄可用）。

当前发行包默认键位（可改 cfg）：

| 武技 | 键位 |
| --- | --- |
| 一字斩·二连 | `ee` |
| 巨型忍者落杀 | `r↑` |
| 寄鹰斩 | `↑l` |
| 寄鹰斩·反向回旋 | `↓l` |
| 苇名十字斩·瞬发 | `r↓`（持防 + 按下方向；cfg 名称含「瞬发」） |
| 仙峰寺菩萨脚 | `↑r` |
| 连击叩拜拳·破魔 | `↓r` |
| 旋风斩 | `e↑` |
| 樱舞 | `rl` |
| 飞渡浮舟 | `l↑` |
| 飞渡漩涡云 | `↑e` |
| 龙闪 | `er` |
| 一心 | `el` |
| 不死斩 | `re` |
| 一字斩 | `ff` |
| 绝技·巨型忍者突刺 | `fl` |

## 忍具（q / t）

- `q` = 游戏内「切换忍具」；`t` =「使用忍具」
- 配置里写 `t`：全场唯一**默认**忍具，按/按住即装备并注入使用；其它忍具结束后会回到它
- 配置里写 `q`：可另绑一个单键忍具，**触发方式与 `t` 相同**（不会成为回默认目标）
- 双键：首键为方向、r/l/e，**尾键为 q/t，或 r/l/e**（方向不能当第二键）；短窗口内「首键 → 尾键」→ 装备并注入使用，尾键按住则持续注入（窗口可在 cfg 用 `# 忍具触发时限` 调整，默认约 0.3s）。例：`↑q`、`↑r`、`→l`、`↓e`。尾键为 r/l/e 时，同帧不会再出同键武技（请避免与武技键位冲突）
- 若已配置裸 `q`，不宜再写 `qt`（单键 `q` 会先触发）
- 未写键位的忍具行忽略

因忍具键与忍具切换键皆有默认忍具，故其不能作为首键，只能作为尾键。且由于防御优先级较高、攻击键容易按出突刺，不建议使用 `r`/`l` 为首键。请写 `↑q` / `→t`，不要写 `q↑`。松开末键后约 **1 秒**锁定（可在键位后加 `-时间` 单独配置，如 `↑q-0.5s`）：不能换其他忍具；再按 `t`/`q` 刷新锁定并继续注入（多段），不回默认；按住期间不倒计时。锁定结束后约 **1.4 秒**回到唯一的 `t` 默认忍具。

当前发行包默认忍具键位：

| 忍具 | 键位 |
| --- | --- |
| 琉璃手里剑 | `t-0s` |
| 老羽雾鸦 | `q-0s` |
| 凤凰紫青伞 | `↓q-0s` |
| 爱哭鬼 | `et-0s` |
| 火舌 | `↑q-0.67s` |
| 螺旋号 | `et-0.67s` |
| 火镰式机关斧 | `↓t-0.5s` |
| 琉璃锈丸 | `↑t-0.5s` |
| 气吹长火筒 | `←t-0.5s` |
| 长效火花 | `→t-0s` |
| 捐赠簿 | `eq-0s` |

## 纸人挂使用说明（纸人上限 / 纸人漂流）

纸人挂是一次性 param 写入器，不会常驻后台。它读取同目录 `battle_instinct.cfg` 中的以下数值：

```ini
纸人上限功能: 开
纸人初始上限: 20
技能增加上限: 2
纸人漂流: 9
```

然后写入 ModEngine 加载的 `gameparam.parambnd.dcx`：

| 字段 | 作用 |
| --- | --- |
| `EquipParamGoods 1000 +0x36` | 纸人基础上限 |
| `EquipParamGoods 1001 +0x36` | 血纸人 / 临时纸人上限 |
| `EquipParamGoods 3800 +0x8C` | 纸人漂流一次给多少临时纸人 |
| `ResourceItemParam goodsId=1000` | 初始纸人、上限成长浮点 |

### 首次安装 / 改数值

1. 确认发行包中的 `dinput8_modengine.dll`、`modengine.ini` 和 `mods\param\gameparam\gameparam.parambnd.dcx` 都已按目录结构放进游戏根目录。
2. 按需修改 `battle_instinct.cfg` 里的 `纸人漂流`；它同时作为临时纸人上限和单次数量。
3. 双击 `纸人挂.bat`，等待提示「完成」。脚本会自动备份 param 文件。
4. 完整退出游戏并重新启动；param 只在启动时读取。
5. 进游戏使用纸人漂流，确认临时纸人上限和单次增加量符合配置。

### 回滚

运行：

```bat
纸人挂.bat revert
```

恢复最近一次 param 备份，然后重启游戏。想关闭整套纸人修正，把 cfg 里的 `纸人上限功能` 改为 `关`；这不会自动回滚已经写入的 param。

### 常见问题

- **纸人漂流没变化**：确认 `dinput8_modengine.dll` 已加载、`mods\param\gameparam\gameparam.parambnd.dcx` 路径正确，并重新运行 `纸人挂.bat` 后重启游戏。
- **ModEngine 冲突**：本 MOD 会自动链式加载 `dinput8_modengine.dll`；不要把自己的 `dinput8.dll` 直接覆盖成 ModEngine，否则本 MOD 不会加载。
- **纸人上限和漂流数值的区别**：纸人上限由 DLL 按 `20 + 2 × i` 实时修正；纸人漂流数量/临时上限才需要 param。
- 字段与踩坑记录见 [docs/纸人参数.md](docs/纸人参数.md)。

## 自行编译（可选）

```bash
cargo build --release
just dist     # 刷新 dist/zh 与 dist/en（含 DLL、cfg、纸人挂和 ModEngine/param 整包）
just bat      # 只重新生成 纸人挂.bat / paperman.bat
just pack     # 打包 battle-instinct_zh.zip / battle-instinct_en.zip
```

产物在 `target/release/`；发行目录为 `dist/zh/`（中文 cfg）与 `dist/en/`（英文 cfg）。
玩家侧按发行包目录结构解压后，双击一次 `纸人挂.bat`（英文包为 `paperman.bat`）即可。

### ModEngine / param 文件说明

发行包已附带：

```text
dinput8_modengine.dll    ← ModEngine 的 dinput8.dll 改名版本
modengine.ini            ← modOverrideDirectory="\mods"
mods\param\gameparam\gameparam.parambnd.dcx
```

`modengine.ini` 保持 `modOverrideDirectory="\mods"`。param 文件来自当前游戏目录中的 Resurrection 参数包，仅用于提供纸人漂流等字段；如你使用其它大型 mod，请用对应版本的 `gameparam.parambnd.dcx` 替换并重跑 `纸人挂.bat`。


## cfg 可调项（纸人 / 心流这几组键名前的 `#` 可有可无，时序类键仍写作 `# 键: 值`）

| 键 | 说明 |
| --- | --- |
| `# 启动信息print窗口: 关` | 启动时是否弹出加载信息窗口（默认关） |
| `# rl触发时限: 0.3s` | **`rl` 樱舞**：按 `r` 后在此时间内按 `l`，且触发时 `r`、`l` 同时按住；真实 B+A 仍被剥离 |
| `# 忍具触发时限: 0.3s` | 双键忍具：首键后须在此时间内按 `q`/`t`（默认约 0.3s） |
| `# 武技排队等待: 0.3s` | 上一招末键松手后，再等此时长才放出排队武技；`0s` = 松手后立刻放 |
| `纸人上限功能: 开` | 纸人上限运行时修正总开关（关 = DLL 不写内存；`纸人挂.bat` 也不写 param） |
| `纸人初始上限: 20` / `技能增加上限: 2` | **只给 `纸人挂.bat` 写 param 用**；DLL 运行时固定为 `20 + 2 × i`，不受这两项影响 |
| `纸人漂流: 9` | 血纸人(临时纸人)上限，也是纸人漂流一次给的数量 |
| `心流功能: 开` | 完美弹反叠层「心流」总开关；`心流持续时间` / `心流最大层数` / `心流回复比例` 见 [docs/心流.md](docs/心流.md) |

> 中英文发行包运行时都按 `20 + 2 × i`；cfg 里的 `纸人初始上限` / `技能增加上限` 只供 param 脚本使用。

忍具多段触发时限写在**键位后**加 `-时间`，如 `↑q-0.5s` 或 `↑q-多段触发时限0.5s`；未写则默认 1s。


## 感谢

- **[dec32](https://github.com/dec32)**：[Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) 原作者——本 MOD 基于其代码与架构
- **[Katalash](https://github.com/katalash)**：[ModEngine](https://github.com/katalash/ModEngine) 原作者——发行包内附带其 DLL 与配置
- 参数包来自本机安装的 **Sekiro: Resurrection**，仅作为纸人漂流 param 覆盖文件随包提供
- [Tmsrise](https://github.com/tmsrise)：[Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon)：[Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo)：原版输入手感测试与改进

第三方组件与打包文件说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 声明

请遵守上游仓库的许可与只狼 MOD 使用惯例。本仓库仅为个人搓招理念下的衍生作品，问题请在本仓库反馈，勿打扰原作者。
