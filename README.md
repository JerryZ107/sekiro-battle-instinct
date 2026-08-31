# Battle Instinct（双键搓招版）

基于 [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct) 修改的只狼武技 / 忍具 MOD。

原版用**方向序列 + 防御/攻击**切换武技；本分支在保留原作者换槽、注入、忍具等核心机制的前提下，**重新设计了武技搓招逻辑**（借助 AI 辅助实现），把武技触发简化为**两个键位**的短窗口连招。

## 与原版的关系

- **上游**：[@dec32](https://github.com/dec32) 的 [Battle Instinct](https://github.com/dec32/sekiro-battle-instinct)
- **本仓库**：理念不同的独立维护版（双键搓招），**不是**上游官方分支
- 忍具仍基本沿用原版方向序列 / ∅ / ⛉ / M4/M5 逻辑

若你更需要原版手感，请直接使用上游发布页。

## 武技搓招（两个键）

键位含义（与改键无关，跟游戏动作绑定）：

| 符号 | 含义 |
|------|------|
| 
 | 防御（鼠标右键 / 格挡） |
| l | 攻击（鼠标左键） |
|  | 动作、(长按)吸引 |
| ↑↓←→ | 移动方向（WASD / 摇杆） |
| f | 连按两次  |
| 
l / 
f / r / l 等 | 两个键的顺序组合 |

双键命中后：换武技槽 → 短暂压制攻击 → 注入防御+攻击放招；第二键按住可继续注入攻击（方便蓄力），松开即停。

默认中文表示例见 [
es/battle_instinct_zh.cfg](res/battle_instinct_zh.cfg)。

## 安装

编译或使用发布包后，将以下文件放到 sekiro.exe 同目录：

1. dinput8.dll
2. attle_instinct.cfg（可用中文表 attle_instinct_zh.cfg 改名）

> [!TIP]
> 若已有其它 dinput8.dll（如 MOD Engine），把**其它** DLL 改名为 dinput8_xxx.dll。本 MOD 会链式加载它们。

### 自行编译

\\ash
cargo build --release
\
## 感谢

- **[dec32](https://github.com/dec32)**：[Battle Instinct](https://github.com/dec32/sekiro-battle-instinct) 原作者——本 MOD 基于其代码与架构
- [Tmsrise](https://github.com/tmsrise)：[Sekiro Weapon Wheel](https://www.nexusmods.com/sekiro/mods/1058)
- [ReaperAnon](https://github.com/ReaperAnon)：[Sekiro Hotkey System](https://www.nexusmods.com/sekiro/mods/1648)
- [Yuzheng Wu](https://github.com/Persona-woo)：原版输入手感测试与改进

## 声明

请遵守上游仓库的许可与只狼 MOD 使用惯例。本仓库仅为个人搓招理念下的衍生作品，问题请在本仓库反馈，勿打扰原作者。
