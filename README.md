# Battle Instinct（双键搓招改版）

本仓库基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改。

原作者实现了只狼武技 / 忍具的**方向序列搓招**与自动换槽。本改版在其工程与换槽、注入、冷却等机制之上，**重新设计了武技搓招逻辑**（借助 AI 完成大量实现与调试），把武技触发简化为：**任意两个键位的组合**即可切槽并放招。

忍具仍沿用原版方向序列方案，未改其核心逻辑。

## 与原版的主要差异

| | 原版 Battle Instinct | 本改版 |
|---|---|---|
| 武技搓招 | 方向序列（如 `↑` / `↑↑`），再按防御+攻击 | **恰好两个键**：方向 / 防 `r` / 攻 `l` / 动作 `f` |
| 放招 | 玩家自己按防御+攻击（部分序列可省略防御） | 命中后自动换槽，再注入防御+攻击 |
| 默认武技 | 无方向时按防御切默认 | 例如 `rl` 等双键绑定（见 cfg） |

键位约定（cfg 最后一列）：

- `r` = 鼠标右键 / 防御
- `l` = 鼠标左键 / 攻击
- `f` = 「动作、(长按)吸引」
- `↑↓←→` = 移动
- `ff` = 连按两次动作键

示例：`r↑`、`↑l`、`fl`、`rf`、`fr`、`↓r`、`rl` 等。

## 安装

将以下文件放到 `sekiro.exe` 同目录：

1. `dinput8.dll`
2. `battle_instinct.cfg`（可用 `res/battle_instinct_zh.cfg` 的中文表）

> [!TIP]
> 若已安装 MOD Engine 或其他 `dinput8.dll`，请把**其他** dll 重命名为 `dinput8_*.dll`。本 MOD 会链式加载它们。

构建：`cargo build --release`，将 `target/release/sekiro_battle_instinct.dll` 改名为 `dinput8.dll`。

## 自定义

编辑 `battle_instinct.cfg`。第一列为 UID，最后列为搓招；武技请使用上述双键写法，忍具仍用方向 / `∅` / `⛉` / `M4`/`M5`。

## 致谢

- **[dec32](https://github.com/dec32)**：[sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 原作者。本改版建立在其代码与思路之上，感谢开源。
- [Tmsrise](https://github.com/tmsrise)：[Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon)：[Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo)：原版输入手感测试与改进

若你需要原版方向序列体验，请使用上游仓库：https://github.com/dec32/sekiro-battle-instinct
