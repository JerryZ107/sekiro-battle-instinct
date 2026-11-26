# 可选 FPS 解锁 / 帧节奏（P2）

默认**关闭**。只有 `sekiro.exe` 同目录存在 `fps_unlock.cfg` 且 `enabled = true` 时才会修改游戏内存；文件不存在时模块只做一次文件探测。

## 配置

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 总开关；也接受 `unlock` |
| `limit` | `60` | 帧率上限，`30..=300`；**必须能整除刷新率** |
| `loading_boost` | `false` | 加载画面把帧间隔写成 `1/300`；会改变加载阶段的节奏 |
| `vsync` | `auto` | `auto` 透传游戏值；`on` 强制 `Present(1)`；`off` 强制 `Present(0)` |
| `speed_fix` | `false` | `limit != 60` 时按上游 `speedFixMatrix` 选最接近 `limit/2` 的常量 |
| `fullscreen_refresh` | `true` | Hook `ChangeDisplaySettingsExW`，阻止游戏全屏时强行改回 60Hz |

示例见 `res/fps_unlock.cfg.example`。

## 刷新率整除规则（最重要）

垂直同步下，帧率上限最好能整除显示器刷新率：

| 刷新率 | 平滑的 limit | 会抽帧的 limit |
| --- | --- | --- |
| 60Hz | 60 | 120、90、144 |
| 180Hz | 60、90、180 | 120、144 |
| 144Hz | 72、144 | 120、90 |

例子：

- 180Hz + 120fps = 1.5 个 vblank/帧，画面按 1、2、1、2 个 vblank 交替 → 平均 120fps 但看起来一顿一顿。
- 180Hz + 60fps = 正好 3 个 vblank/帧 → 和原作一样顺。
- 60Hz + 60fps = 1:1 → 顺。

模块启动时会读取当前刷新率并写日志：

```text
fps unlock: limit=60 divides display 180Hz (even vblank pacing)
fps unlock: limit=120 does NOT divide display 180Hz; v-sync judder is likely - use 180 or 90
```

## 安全门禁

- 只支持 Sekiro 1.6.0.0 的固定 RVA；写入前会读当前 `fps_offset`，必须约等于 `1/60`。
- 如果检测到帧间隔已被其他解锁 MOD 改过，直接拒绝并写日志，不与其抢写。
- `speed_fix` 会先读取 RIP 相对位移指向的当前常量，只有它落在已知 `speedFixMatrix` 里才打补丁；否则跳过。
- 所有代码/数据写入都经过 `VirtualProtect` + `WriteProcessMemory`，并 `FlushInstructionCache`。
- 启动日志会明确写出 `fps unlock: active ...` 或 `fps unlock: refused ...`。

## 已知限制

- `>60fps` 会改变游戏逻辑/动画速度，`speed_fix` 只是近似补偿，不保证所有招式/敌人行为正常。
- `fullscreen_refresh` 只 Hook 了 `ChangeDisplaySettingsExW`；没有实现上游的 `ResizeTarget` 内部刷新率 shellcode，独占全屏仍可能被锁 60Hz。建议配合无边框窗口。
- 如果全屏刷新率仍然在 60/180 之间跳变，最稳的是游戏内切 **窗口化/无边框**，不要让游戏修改显示模式。
- `vsync = off` 在窗口模式下仍可能被 DWM 同步；只有独占全屏/允许撕裂的交换链才有明显效果。
- `fps_unlock.cfg` 只在启动时读取一次；修改后需要重启游戏。
- `safe_mode.enable` 优先级更高：存在时 `fps_unlock` 也不会初始化。

## 与帧率诊断配合

开启 `enable_frame_perf.enable` 后，`frame_timing.log` 里的 `present_sync0` 可以直接看到强制关垂直同步的帧；`present_fps` 与 `fps` 的差异可以判断解锁是否真正生效。
