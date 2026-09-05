# Battle Instinct（战斗本能）（双键搓招版）

![演示](demo.gif)

基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改的只狼武技 / 忍具 MOD。原版用**方向序列 + 防御/攻击**切换武技；本分支在保留原作者换槽、注入等核心机制的前提下，**重新设计了武技搓招与忍具触发逻辑**（借助 AI 辅助实现）：武技为**两个键位**短窗口连招；忍具为 **首键(方向/攻防/交互) + q/t**，或单独 `t` / `q`，直接装备并注入使用。

A Sekiro combat-art / prosthetic MOD forked from [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct). Upstream switches arts with **direction sequences + block/attack**. This fork keeps the original slot-swap / inject core, but **redesigns art combos and prosthetic triggers** (AI-assisted): arts use a **two-key** short window; prosthetics use **first key (move/block/attack/interact) + q/t**, or bare `t` / `q`, to equip and inject use.

## 直接使用（推荐） / Quick start (recommended)

**只需两个文件**复制到 `sekiro.exe` 同目录：`dinput8.dll` + `battle_instinct.cfg`。若已有其它 `dinput8.dll`（如 MOD Engine），把**原来的**改名为 `dinput8_xxx.dll`；本 MOD 会链式加载它们。**不需要** `version.dll`。

Copy **only two files** next to `sekiro.exe`: `dinput8.dll` + `battle_instinct.cfg`. If another `dinput8.dll` exists (e.g. MOD Engine), rename **that** one to `dinput8_xxx.dll`; this MOD chain-loads it. **`version.dll` is not required.**

### 中文发行包 / Chinese release

- [dist/zh/dinput8.dll](dist/zh/dinput8.dll)
- [dist/zh/battle_instinct.cfg](dist/zh/battle_instinct.cfg)

### 英文发行包 / English release

- [dist/en/dinput8.dll](dist/en/dinput8.dll)
- [dist/en/battle_instinct.cfg](dist/en/battle_instinct.cfg)

### cfg 可调项 / Config toggles (comments at top of cfg)

| 中文 | English | 说明 |
| --- | --- | --- |
| `# 启动信息print窗口: 关` | `# boot console: off` | 启动时是否弹出加载信息窗口（默认关） |
| `# rl触发时限: 0.1s` | `# rl window: 0.1s` | 仅 `rl` 搓招：`r` 后须在此时间内按 `l` |

忍具多段触发时限写在**键位后**加 `-时间`，如 `↑q-0.5s` 或 `↑q-多段触发时限0.5s`；未写则默认 1s。Each prosthetic bind can append `-time` for multi-hit lock, e.g. `↑q-0.5s` or `↑q-multi-hit0.5s`; default 1s if omitted.

## 与原版的关系 / Relation to upstream

- **上游**：[@dec32](https://github.com/dec32) 的 [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **本仓库**：理念不同的独立维护版（双键搓招 + 忍具 q/t），**不是**上游官方分支

若你更需要原版手感，请直接使用上游发布页。

- **Upstream**: [@dec32](https://github.com/dec32)’s [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **This repo**: independently maintained (two-key arts + q/t prosthetics), **not** an official upstream branch

If you prefer the original feel, use the upstream release instead.

## 武技搓招（两个键） / Combat arts (two keys)

键位含义跟游戏动作绑定，改键后仍有效。双键命中后：换武技槽 → 短暂压制攻击 → 注入防御+攻击放招；第二键按住可继续注入防御+攻击（方便蓄力），松开即停。当前招持续注入时再搓出的招会排队，松手后放出。默认检测窗口约 **0.3s**；**首键为 `l`**（如 `l↑`）时约 **0.7s**；**`rl` 樱舞** 默认约 **0.1s**（可在 cfg 调整）。武技侧不建议以 `r`/`l` 为首键：防御优先级较高，攻击键容易按出突刺。源码：`res/battle_instinct_zh.cfg`（中文）、`res/battle_instinct.cfg`（英文）；发行包：`dist/zh/`、`dist/en/`。

Keys follow in-game actions (remaps still work). On a two-key match: swap art slot → briefly suppress attack → inject block+attack; hold the second key to keep injecting attack (charge), release to stop. Combos input while an art is still injecting are queued and fire after release. Default detect window is about **0.3s**; about **0.7s** when the first key is `l` (e.g. `l↑`); **`rl` (Sakura Dance)** defaults to about **0.1s** (cfg-tunable). Avoid `r`/`l` as the first key: block has high priority; attack easily thrusts. Sources: `res/battle_instinct_zh.cfg` (ZH), `res/battle_instinct.cfg` (EN); releases: `dist/zh/`, `dist/en/`.

| 符号(Symbol) | 含义(Meaning) |
| --- | --- |
| 右键(Right mouse) `r` | 防御 / 格挡 (Block) |
| 左键(Left mouse) `l` | 攻击 (Attack) |
| 动作(Interact) `e` | 动作、(长按)吸引 (Interact / hold to beckon) |
| 方向箭头(Arrows) | 移动（WASD / 摇杆）(Move: WASD / stick) |
| `ee` | 连按两次动作键 (Double-tap interact) |
| `rl` / `re` / `er` / `el` 等 | 两个键的顺序组合 (Ordered two-key pairs)；同一行可用 `/` 写多种搓法，如 `↓l/l↓` |

当前发行包默认键位（可改 cfg）/ Default release binds (editable in cfg):

| 武技(Combat Art) | 键位(Bind) |
| --- | --- |
| 一字斩·二连 (Ichimonji: Double) | `ee` |
| 巨型忍者落杀 (Shadowfall) | `re` |
| 寄鹰斩 (Nightjar Slash) | `↑l` |
| 寄鹰斩·反向回旋 (Nightjar Slash Reversal) | `↓l` / `l↓` |
| 苇名十字斩 (Ashina Cross) | `↓r` |
| 仙峰寺菩萨脚 (High Monk) | `↑r` |
| 连击叩拜拳·破魔 (Praying Strikes - Exorcism) | `r↑` |
| 旋风斩 (Whirlwind Slash) | `e↑` |
| 樱舞 (Sakura Dance) | `rl` |
| 飞渡浮舟 (Floating Passage) | `l↑` |
| 飞渡漩涡云 (Spiral Cloud Passage) | `↑e` |
| 龙闪 (Dragon Flash) | `er` |
| 一心 (One Mind) | `el` |
| 不死斩 (Empowered Mortal Draw) | `r↓` |

## 忍具（q / t） / Prosthetics (q / t)

- `q` = 游戏内「切换忍具」；`t` =「使用忍具」
- 配置里写 `t`：全场唯一**默认**忍具，按/按住即装备并注入使用；其它忍具结束后会回到它
- 配置里写 `q`：可另绑一个单键忍具，**触发方式与 `t` 相同**（不会成为回默认目标）
- 双键：首键为方向、r/l/e，**尾键只能是 q 或 t**（方向不能当第二键）；短窗口内「首键 → 尾键」→ 装备并注入使用，尾键按住则持续注入
- 若已配置裸 `q`，不宜再写 `qt`（单键 `q` 会先触发）
- 未写键位的忍具行忽略

因忍具键与忍具切换键皆有默认忍具，故其不能作为首键，只能作为尾键。且由于防御优先级较高、攻击键容易按出突刺，不建议使用 `r`/`l` 为首键。请写 `↑q` / `→t`，不要写 `q↑`。松开末键后约 **1 秒**锁定（可在键位后加 `-时间` 单独配置，如 `↑q-0.5s`）：不能换其他忍具；再按 `t`/`q` 刷新锁定并继续注入（多段），不回默认；按住期间不倒数。锁定结束后约 **1.4 秒**回到唯一的 `t` 默认忍具。

- `q` = in-game “Switch Prosthetic”; `t` = “Use Prosthetic”
- Bare `t`: unique **default** tool — press/hold to equip and inject use; other tools return to it afterward
- Bare `q`: optional second one-key tool with the **same fire style as `t`** (not the return-default target)
- Two-key: first is move/r/l/e; **tail must be q or t** (directions cannot be second); within a short window, first→tail equips and injects use; hold the tail to sustain
- If bare `q` is set, avoid `qt` (bare `q` fires first)
- Prosthetic lines without a bind token are ignored

Because both Use (`t`) and Switch (`q`) have default tools, they cannot be first keys — only tails. Also avoid `r`/`l` as first keys: block has high priority; attack easily thrusts. Prefer `↑q` / `→t`; never write `q↑`. After releasing the tail key, about **1s** of lock by default (override per bind with `-time`, e.g. `↑q-0.5s`): cannot switch tools; pressing `t`/`q` again refreshes the lock and keeps injecting (multi-hit). After lock ends, about **1.4s** later it returns to the bare-`t` default.

当前发行包默认忍具键位 / Default release prosthetic binds:

| 忍具(Prosthetic) | 键位(Bind) |
| --- | --- |
| 琉璃手里剑 (Lazulite Shuriken) | `t-0s` |
| 老羽雾鸦 (Aged Feather Mist Raven) | `q-0s` |
| 朱雀红莲伞 (Suzaku's Lotus Umbrella) | `↓q-0s` |
| 凤凰紫青伞 (Phoenix's Lilac Umbrella) | `↓t-0s` |
| 爱哭鬼 (Mountain Echo) | `et-0s` |
| 火舌 (Leaping Flame) | `↑t-0.67s` |
| 螺旋号 (Spiral Spear) | `et-0.67s` |
| 火镰式机关斧 (Sparking Axe) | `←q-0.5s` |
| 琉璃锈丸 (Lazulite Sabimaru) | `↑q-0.5s` |
| 气吹长火筒 (Okinaga's Flame Vent) | `←t-0.5s` |
| 长效火花 (Long Spark) | `→q-0s` |
| 捐赠簿 (Finger Whistle / Divine Abduction bind) | `eq-0s` |

## 自行编译（可选） / Build yourself (optional)

```bash
cargo build --release
just dist   # 刷新 dist/zh 与 dist/en（各含 dinput8.dll + battle_instinct.cfg）
```

产物在 `target/release/`；发行目录为 `dist/zh/`（中文 cfg）与 `dist/en/`（英文 cfg）。

```bash
cargo build --release
just dist   # refresh dist/zh and dist/en (dinput8.dll + battle_instinct.cfg each)
```

Output: `target/release/`; release folders `dist/zh/` (Chinese cfg) and `dist/en/` (English cfg).

## 感谢 / Credits

- **[dec32](https://github.com/dec32)**：[Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) 原作者——本 MOD 基于其代码与架构
- [Tmsrise](https://github.com/tmsrise)：[Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon)：[Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo)：原版输入手感测试与改进

- **[dec32](https://github.com/dec32)**: [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) original author — this MOD is based on their code and architecture
- [Tmsrise](https://github.com/tmsrise): [Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon): [Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo): input-feel testing and improvements on the original

## 声明 / Disclaimer

请遵守上游仓库的许可与只狼 MOD 使用惯例。本仓库仅为个人搓招理念下的衍生作品，问题请在本仓库反馈，勿打扰原作者。

Please follow the upstream license and Sekiro MOD conventions. This repo is a personal derivative focused on a different combo philosophy; report issues here, and do not bother the original author.
