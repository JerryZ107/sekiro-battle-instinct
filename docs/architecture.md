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
- `frame_perf` 改为 opt-in：默认不启动线程/ring/日志；需要排查时在游戏目录创建 `enable_frame_perf.enable`。
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
- 玩家感知窗口全部按当前 FPS 缩放：`ArtComboWindow` 的 18/18/42 帧、`rl` 窗口、忍具两键窗口、排队延迟、忍具多段锁、心流持续时间。
- 注入类帧数（`BLOCK_INJECTION_DURATION`、`ATTACK_SUPRESSION_DURATION`、`PROSTHETIC_SUPPRESSION_DURATION`、`tool_min_use`）保持游戏原生帧，不做墙钟缩放，与上游 `dec32/sekiro-battle-instinct` 一致。
- 配置中的 `*_frames` 字段仍按 60fps 存储；`Config::rl_window()/tool_window()/art_queue_delay()/tool_lock()` 返回 `Frames`，在计时器启动时再换算，避免启动阶段采样器尚未就绪时把窗口锁死在 60fps。

### Present 侧帧节奏诊断

- `frame_perf::record_present(sync)` 在每个 `Present`/`Present1` 入口记录时间戳，输出 `present_fps / present_avg_ms / present_p99_ms / present_max_ms / present_gt16.7 / present_gt33 / present_ui / present_sync0`。
- `tick` 与 `present` 分别聚合：如果 `fps` 正常但 `present_max_ms` 高，说明渲染/垂直同步侧卡顿；如果 `present_fps` 正常但 `mod_max_us` 高，才是 MOD 自身开销。
- `ResizeBuffers` 通过 generation 重置 Present 时钟，避免把 swap-chain 重建间隔误报成一次超长帧；超过 25ms 的 Present 仍会以 `LONG ... src=present` 单独记录。

## 启动诊断与安全模式

- 启动阶段写入带步骤号的日志：`dll: DllMain ...`、`init[1]` 到 `init[11]`，分别对应帧率诊断、等待游戏代码、创建 Mod（含手柄初始化）、伤害钩子、输入钩子、菜单钩子、UI、MinHook 启用等阶段。
- 12 秒 watchdog：如果 `init` 在 12 秒内没有完成，会写出最后停留的步骤号，便于区分是 `wait_for_game_code`、`Mod::new`（gilrs 手柄枚举）还是 hook 安装卡住。
- `safe_mode.enable`：只加载 DLL、写日志，跳过 chainload、所有 hook、手柄和 UI。用于确认 DLL 本身能否被游戏加载，不改变游戏行为。
- `no_gamepad.enable`：跳过 `gilrs` 初始化，键盘玩家可用；也是排查启动卡死的第一候选开关。
- 以上 marker 都放在 `sekiro.exe` 同目录。

## 可选 FPS 解锁（P2）

- 新模块 `src/fps_unlock.rs`，默认惰性：只有游戏目录存在 `fps_unlock.cfg` 且 `enabled = true` 才初始化。
- 功能：帧率上限（`fps_offset`）、加载 300fps、`Present` 垂直同步覆盖、高帧速度补偿、`ChangeDisplaySettingsExW` 刷新率 Hook。
- 安全：写入前校验当前帧间隔是否为 `1/60`；速度补偿先校验目标常量是否在已知表中；所有写入走 `VirtualProtect` + `WriteProcessMemory`。
- `Present` 入口调用 `fps_unlock::on_present(sync)`，返回值再传给原始 `Present`；关闭时只是一次原子读。
- 配置与风险说明见 `docs/fps-unlock.md`。
