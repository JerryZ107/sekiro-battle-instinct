# 可选 FPS 解锁 / 帧节奏（P2）

默认**关闭**。只有 `sekiro.exe` 同目录存在 `fps_unlock.cfg` 且 `enabled = true` 时才会修改游戏内存；文件不存在时模块只做一次文件探测。

## 配置

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 总开关；也接受 `unlock` |
| `limit` | `120` | 帧率上限，`30..=300` |
| `loading_boost` | `true` | 加载画面把帧间隔写成 `1/300`，缩短加载时间 |
| `vsync` | `auto` | `auto` 透传游戏值；`on` 强制 `Present(1)`；`off` 强制 `Present(0)` |
| `speed_fix` | `true` | `limit != 60` 时按上游 `speedFixMatrix` 选最接近 `limit/2` 的常量 |
| `fullscreen_refresh` | `false` | Hook `ChangeDisplaySettingsExW`，清掉 `DM_DISPLAYFREQUENCY` |

示例见 `res/fps_unlock.cfg.example`。

## 安全门禁

- 只支持 Sekiro 1.6.0.0 的固定 RVA；写入前会读当前 `fps_offset`，必须约等于 `1/60`。
- 如果检测到帧间隔已被其他解锁 MOD 改过，直接拒绝并写日志，不与其抢写。
- `speed_fix` 会先读取 RIP 相对位移指向的当前常量，只有它落在已知 `speedFixMatrix` 里才打补丁；否则跳过。
- 所有代码/数据写入都经过 `VirtualProtect` + `WriteProcessMemory`，并 `FlushInstructionCache`。
- 启动日志会明确写出 `fps unlock: active ...` 或 `fps unlock: refused ...`。

## 已知限制

- `>60fps` 会改变游戏逻辑/动画速度，`speed_fix` 只是近似补偿，不保证所有招式/敌人行为正常。
- `fullscreen_refresh` 只 Hook 了 `ChangeDisplaySettingsExW`；没有实现上游的 `ResizeTarget` 内部刷新率 shellcode，独占全屏仍可能被锁 60Hz。建议配合无边框窗口。
- `vsync = off` 在窗口模式下仍可能被 DWM 同步；只有独占全屏/允许撕裂的交换链才有明显效果。
- `fps_unlock.cfg` 只在启动时读取一次；修改后需要重启游戏。
- `safe_mode.enable` 优先级更高：存在时 `fps_unlock` 也不会初始化。

## 与帧率诊断配合

开启 `enable_frame_perf.enable` 后，`frame_timing.log` 里的 `present_sync0` 可以直接看到强制关垂直同步的帧；`present_fps` 与 `fps` 的差异可以判断解锁是否真正生效。
