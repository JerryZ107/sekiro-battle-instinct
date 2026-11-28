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

## 帧路径优化（第二轮）

- `ActionFrame` 每帧只从输入读取一次，统一计算 11 个 token 的 `held/pressed`；`ArtComboWindow` 和 `ProstheticState` 共用，不再各自维护一套边沿状态。
- `InventorySnapshot` 增加无锁原子镜像：`Emblem::tick` 不再每帧获取库存互斥锁；伤害路径的纸人库存读取也走原子快照。
- 未连接手柄时，`gilrs` 只每 500ms 探测一次重连，不再每帧泵 HID 事件。
- Overlay renderer 初始化失败后设置 `RENDERER_FAILED`，避免每次 Present 重试 D3DCompile；`ResizeBuffers` 或打开 UI 时允许重试。
- 伤害钩子缓存最近一次已校验的 `damage_info` 指针，避免连续受击重复做内存探针。

## 第三轮：上游输入/配置/初始化迁移

- `DirectionBuffer` 移植上游摇杆阈值：`0.85` 普通、`0.4` 反向弹、`0.9` 旋转，并保留 neutral 状态，避免摇杆漂移产生伪方向边沿。
- `ComboTable<K,T>` 以 8-bit 索引的固定数组替代 `HashMap<ArtCombo/ToolCombo, UID>`；instant art 改为按当前边沿生成候选组合后直接查表，不再扫描全部绑定。
- 忍具已装备在其他槽位时，优先调用原生 `set_equipped_prosthetic` 激活该槽，而不是交换背包行。
- `game.rs` 统一 `resolve_pointer_chain`，并缓存 PlayerIns / SprjChrDataModule，菜单关闭时失效。
- 启动阶段只读取/解析一次 `battle_instinct.cfg`，同一份文本传给 Emblem/Flow/PaperGuard。
- Overlay 缓存 swap-chain 尺寸和编译后的 shader bytecode，ResizeBuffers 重建 renderer 时不再重复 `D3DCompile`。

## 第四轮：帧率自适应计时

- `frame.rs` 的 `Frames::actual_at()` 改为 `u32` 运算并饱和，避免 `240` 帧 @ 300fps 这类长计时溢出 `u16`；`as_actual()` 统一四舍五入。
- `Framerate` 不再在 30 个采样后永久冻结：每 60 帧继续采样，并跳过 `> max(3 × 预期, 2s)` 的暂停/加载窗口，支持游戏中途开关 FPS 解锁。
- 玩家感知窗口全部按当前 FPS 缩放：`ArtComboWindow` 的 18/18/42 帧、三键中键→尾键 60 帧、`rl` 窗口、忍具两键窗口、排队延迟、忍具多段锁、心流持续时间。
- 注入类帧数（`BLOCK_INJECTION_DURATION`、`ATTACK_SUPRESSION_DURATION`、`PROSTHETIC_SUPPRESSION_DURATION`、`tool_min_use`）保持游戏原生帧，不做墙钟缩放，与上游 `dec32/sekiro-battle-instinct` 一致。
- 配置中的 `*_frames` 字段仍按 60fps 存储；`Config::rl_window()/tool_window()/art_queue_delay()/tool_lock()` 返回 `Frames`，在计时器启动时再换算，避免启动阶段采样器尚未就绪时把窗口锁死在 60fps。

## 启动诊断与安全模式

- 启动阶段写入带步骤号的日志：`dll: DllMain ...`、`init[1]` 到 `init[11]`，分别对应启动诊断、等待游戏代码、创建 Mod（含手柄初始化）、伤害钩子、输入钩子、菜单钩子、UI、MinHook 启用等阶段。
- 12 秒 watchdog：如果 `init` 在 12 秒内没有完成，会写出最后停留的步骤号，便于区分是 `wait_for_game_code`、`Mod::new`（gilrs 手柄枚举）还是 hook 安装卡住。
- `safe_mode.enable`：只加载 DLL、写日志，跳过 chainload、所有 hook、手柄和 UI。用于确认 DLL 本身能否被游戏加载，不改变游戏行为。
- `no_gamepad.enable`：跳过 `gilrs` 初始化，键盘玩家可用；也是排查启动卡死的第一候选开关。
- 以上 marker 都放在 `sekiro.exe` 同目录。

## 已移除的实验性 FPS 解锁

- 游戏侧 `fps_offset` / `speedfix` 补丁容易与显示器刷新率、垂直同步、全屏模式冲突（例如 180Hz 下 120fps 会 3:2 抽帧），已从主线移除。
- 同时移除 `frame_perf` 采样/诊断；只保留 P0 的 MOD 内部计时缩放，它不写游戏内存、不改 `Present` 同步参数、不改刷新率。

## UI/Overlay Ablation Test

- `disable_overlay.enable`: skip `ui::start`, so no `Present`, `ResizeBuffers`, `Present1`, `GetRawInputData`, `SetCursorPos`, `ClipCursor` or `WndProc` hooks and no CPU painter.
- Combat hooks (`process_input`, damage reaction, menu dialog) stay enabled.
- Use it to distinguish overlay/Present-path stutter from the game/Windows exclusive-fullscreen path: if the stutter disappears, investigate the overlay; if it remains, the display path is responsible.

## 自动 Alt+Tab 窗口重置

- 启动完成后 6 秒，后台线程找到当前进程的 `CSclass` 窗口，执行一次 `SW_MINIMIZE` -> 300ms -> `SW_RESTORE` + `SetForegroundWindow`。
- 效果等价于手动 Alt+Tab 切出再切回，用于打破 Sekiro 独占全屏下的显示通路卡顿。
- 默认启用；游戏目录放 `disable_auto_alt_tab.enable` 可关闭。
- 不修改帧率上限、Present 同步参数或显示刷新率，只改变窗口前后台状态。
