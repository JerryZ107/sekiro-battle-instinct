# Battle Instinct（战斗本能）（双键搓招版）

基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改的只狼武技 / 忍具 MOD。

原版用**方向序列 + 防御/攻击**切换武技；本分支在保留原作者换槽、注入、忍具等核心机制的前提下，**重新设计了武技搓招逻辑**（借助 AI 辅助实现），把武技触发简化为**两个键位**的短窗口连招。

## 直接使用（推荐）

仓库已包含编译好的中文发行包，**无需自己编译**：

- [dist/zh/dinput8.dll](dist/zh/dinput8.dll)
- [dist/zh/battle_instinct.cfg](dist/zh/battle_instinct.cfg)（中文搓招表）

把上面两个文件复制到 `sekiro.exe` 同目录即可。

> [!TIP]
> 若已有其它 `dinput8.dll`（如 MOD Engine），把**其它** DLL 改名为 `dinput8_xxx.dll`。本 MOD 会链式加载它们。

## 与原版的关系

- **上游**：[@dec32](https://github.com/dec32) 的 [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **本仓库**：理念不同的独立维护版（双键搓招），**不是**上游官方分支
- 忍具仍基本沿用原版方向序列、空输入默认、格挡+忍具、鼠标侧键逻辑

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

双键命中后：换武技槽 → 短暂压制攻击 → 注入防御+攻击放招；第二键按住可继续注入攻击（方便蓄力），松开即停。

源码侧中文表示例见 [res/battle_instinct_zh.cfg](res/battle_instinct_zh.cfg)。

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
