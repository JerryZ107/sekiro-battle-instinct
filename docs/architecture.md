# 架构说明（重构后）

本项目按五个用户功能拆分，`core.rs` 只保留每帧输入编排和共享状态。

| 功能 | 模块 | 说明 |
| --- | --- | --- |
| 1. 流派招式键位 | `features/combat_art.rs` | 招式 UID、fallback、swap-out 计时、Action 注入状态、队列状态 |
| 2. 义手忍具键位 | `features/prosthetic.rs` | 忍具槽位、装备/回滚、first-key/tail 状态机、USE 注入状态 |
| 3. 纸人上限 | `features/emblem_cap.rs` + `inventory.rs` | 从共享背包快照读取 5 个上限技能，只在新快照上写 `PlayerData+0x146` |
| 4. 纸人替伤 | `features/paper_guard.rs` + `inventory.rs` | 只读缓存纸人库存，从缓存行扣纸人，零纸人不再触发全量扫描 |
| 5. 心流 | `features/flow.rs` | 完美弹反事件队列、层数/超时状态、架势返还 |

## 共享底层

- `game.rs`：游戏指针链、`UID`/`ItemID`、`set_slot`、`ID` trait。
- `memory.rs`：上游风格的 `ReadProcessMemory` 探针与整块读取。
- `inventory.rs`：一次遍历同时收集纸人库存和上限技能，按背包基址/菜单代际失效。
- `action.rs`：Sekiro action bitfields 和 input 三个 word 的读写助手。
- `menu_probe.rs`：菜单状态后台缓存；普通读取不再每帧 `VirtualQuery`。

## 借鉴的上游模式

- `dec32/sekiro-battle-instinct`：`game.rs` 集中游戏 ID/指针/原生函数；功能代码只消费解析后的值；输入缓冲区使用紧凑表示。
- `kargoshakar/SekiroImGuiCustom`：`IsValidReadPtr` 式探针、worker 线程一次性初始化、静态偏移缓存、只在校验/状态边沿更新。

## 扫描策略

- 普通路径：`inventory::ensure_fresh()` 只在背包基址或菜单代际变化时扫描。
- 伤害路径：`inventory::ensure_fresh_for_damage()` 最多每 1 秒兜底刷新一次，用于发现世界拾取。
- 扣纸人：`inventory::spend_emblems()` 只使用缓存行，不扫描。
- 启动阶段：第一次可用背包只扫描一次；`Emblem` 和 `PaperGuard` 共享同一份快照。
