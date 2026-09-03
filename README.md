# Battle Instinct（战斗本能）（双键搓招版）

基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改的只狼武技 / 忍具 MOD。

![演示](demo.gif)

原版用**方向序列 + 防御/攻击**切换武技；本分支在保留原作者换槽、注入等核心机制的前提下，**重新设计了武技搓招与忍具触发逻辑**（借助 AI 辅助实现）：武技为**两个键位**短窗口连招；忍具为 **首键(方向/攻防/交互/q) + q/t** 或单独 `t`，直接装备并注入使用。

## 直接使用（推荐）

仓库已包含编译好的中文发行包，**无需自己编译**：

- [dist/zh/dinput8.dll](dist/zh/dinput8.dll)
- [dist/zh/battle_instinct.cfg](dist/zh/battle_instinct.cfg)（中文搓招表）

把上面两个文件复制到 `sekiro.exe` 同目录即可。

> [!TIP]
> 若已有其它 `dinput8.dll`（如 MOD Engine），把**其它** DLL 改名为 `dinput8_xxx.dll`。本 MOD 会链式加载它们。

## 与原版的关系

- **上游**：[@dec32](https://github.com/dec32) 的 [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **本仓库**：理念不同的独立维护版（双键搓招 + 忍具 q/t），**不是**上游官方分支

若你更需要原版手感，请直接使用上游发布页。

## 武技搓招（两个键）

键位含义（跟游戏动作绑定，改键后仍有效）：

| 符号 | 含义 |
| --- | --- |
| 右键 r | 防御 / 格挡 |
| 左键 l | 攻击 |
| 动作 f | 动作、(长按)吸引 |
| 方向箭头 | 移动（WASD / 摇杆） |
| ff | 连按两次动作键 |
| rl / rf / fr / fl 等 | 两个键的顺序组合 |

双键命中后：换武技槽 → 短暂压制攻击 → 注入防御+攻击放招；第二键按住可继续注入攻击（方便蓄力），松开即停。当前招持续注入时再搓出的招会排队，松手后放出。

默认检测窗口约 **0.3s**；**首键为 `l`**（如 `l↑`）时约 **0.7s**。

当前发行包默认键位（可改 cfg）：

| 武技 | 键位 |
| --- | --- |
| 一字斩·二连 | `ff` |
| 巨型忍者落杀 | `fl` |
| 寄鹰斩 | `↑l` |
| 寄鹰斩·反向回旋 | `↓l` |
| 苇名十字斩 | `↓r` |
| 仙峰寺菩萨脚 | `f↑` |
| 连击叩拜拳·破魔 | `r↑` |
| 旋风斩 | `↑r` |
| 樱舞 | `rl` |
| 飞渡浮舟 | `l↑` |
| 飞渡漩涡云 | `↑f` |
| 龙闪 | `rf` |
| 一心 | `fr` |
| 不死斩 | `lf` |

源码侧中文表：[res/battle_instinct_zh.cfg](res/battle_instinct_zh.cfg)；发行包：[dist/zh/battle_instinct.cfg](dist/zh/battle_instinct.cfg)。

> [!NOTE]
> 武技侧也不建议以 `r`/`l` 为首键：防御优先级较高，攻击键容易按出突刺。

## 忍具（q / t）

- `q` = 游戏内「切换忍具」；`t` =「使用忍具」
- 配置里写 `t`：全场唯一**默认**忍具，按/按住即装备并注入使用；其它忍具结束后会回到它
- 配置里写 `q`：可另绑一个单键忍具，**触发方式与 `t` 相同**（不会成为回默认目标）
- 双键：首键为方向、r/l/f，**尾键只能是 q 或 t**（方向不能当第二键）；短窗口内「首键 → 尾键」→ 装备并注入使用，尾键按住则持续注入
- 若已配置裸 `q`，不宜再写 `qt`（单键 `q` 会先触发）
- 未写键位的忍具行忽略

**为何尾键固定为 q/t（不要写 `q↑`）**  
因忍具键与忍具切换键皆有默认忍具，故其不能作为首键，只能作为尾键。且由于防御优先级较高、攻击键容易按出突刺，不建议使用 `r`/`l` 为首键。请写 `↑q` / `→t`，不要写 `q↑`。

松开末键后约 **1 秒**锁定：不能换其他忍具；再按 `t`/`q` 刷新锁定并继续注入（多段），不回默认。按住期间不倒数。锁定结束后约 **1.4 秒**回到唯一的 `t` 默认忍具。

当前发行包默认忍具键位：

```
70500 琉璃手里剑         t
74100 老羽雾鸦           q
76200 朱雀红莲伞         ↓q
76300 凤凰紫青伞         ↓t
79200 爱哭鬼             →t
78400 火舌               ↑q
78300 螺旋号             ft
73200 火镰式机关斧       ←q
75300 琉璃锈丸           ↑t
72200 气吹长火筒         ←t
71200 长效火花           →q
77200 捐赠簿             fq
```

## 自行编译（可选）

```bash
cargo build --release
```

产物一般在 `target/release/`，也可更新到 `dist/zh/`。

## 感谢

- **[dec32](https://github.com/dec32)**：[Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) 原作者——本 MOD 基于其代码与架构
- [Tmsrise](https://github.com/tmsrise)：[Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon)：[Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo)：原版输入手感测试与改进

## 声明

请遵守上游仓库的许可与只狼 MOD 使用惯例。本仓库仅为个人搓招理念下的衍生作品，问题请在本仓库反馈，勿打扰原作者。
