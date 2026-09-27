//----------------------------------------------------------------------------
//
//  Paper-doll (紙人 / Spirit Emblem) cap fix
//
//  背景：纸人上限是存在存档里的（学技能时由游戏加一次），所以只改 param 只能影响
//  新建存档；老存档需要运行时把上限改掉。本模块：
//    1) 探针模式：把 PlayerData 里等于「HUD 上限 / HUD 数量」的字段、以及 InventoryData
//       头部的 (id, count) 表打进 battle_instinct.log，用来定位字段；
//    2) 修正模式：定位后按 基础上限 + 每个上限技能加成 × 已习得技能数 写回上限。
//  未配置偏移时只做 1)，绝不写内存。
//
//----------------------------------------------------------------------------

use std::{collections::HashSet, fs, path::Path};

use crate::game;

const DEFAULT_BASE_CAP: u32 = 25;
const DEFAULT_PER_SKILL: u32 = 5;
const DEFAULT_LEGACY_BASE: u32 = 15;
const DEFAULT_LEGACY_PER: u32 = 1;
const DEFAULT_SKILL_COUNT: u32 = 5;
const DEFAULT_WINDOW: usize = 0x1000;
const INVENTORY_SCAN_LEN: usize = 0x2000;
/// 增量扫描总量上限（约 2GB；每帧 1MB，找到即停）
const SCAN_MAX_BYTES: usize = 0x8000_0000;
/// 关注的物品 id：1000 纸人、1001 临时纸人、1020 纸人上限(+0x1C 引用)、1021 临时上限
const WATCH_ITEM_IDS: [u32; 4] = [1000, 1001, 1020, 1021];

pub struct Emblem {
    enabled: bool,
    base_cap: u32,
    per_skill: u32,
    legacy_base: u32,
    legacy_per: u32,
    skill_count: u32,
    /// cfg 直接指定的已学技能数，<0 表示自动
    learned: i32,
    /// PlayerData 内偏移，0 = 未定位
    cap_offset: usize,
    cap_width: usize,
    /// 已学技能数所在的 PlayerData 偏移，0 = 不读
    learned_offset: usize,
    /// 可选还原写入（把之前写错的字段改回原值）：0 = 不用
    restore_offset: usize,
    restore_value: u32,
    /// 纸人漂流：血纸人(临时纸人)上限，也是一次给的数量
    drift: u32,
    /// 初始纸人（开局携带数量，默认 30）
    initial_paper: u32,
    /// 已定位到的 param 行地址（内存里直接改，玩家无需改 param 文件）
    param_rows: Option<ParamRows>,
    param_scans: u32,
    /// 是否允许 DLL 去内存里定位并改写 param（默认关：扫描代价大，容易卡顿）
    param_patch_enabled: bool,
    // 增量扫描状态
    scan_pass: u8,
    scan_addr: usize,
    scan_end: usize,
    scan_off: usize,
    scan_ok: bool,
    scan_done: bool,
    scanned: usize,
    diag: [u32; 4],
    probe: bool,
    probe_cap: u32,
    probe_count: u32,
    window: usize,
    ticks: u64,
    reported: HashSet<usize>,
    snapshot: Vec<u8>,
    inventory_dumped: bool,
    candidates_logged: u32,
    deltas_logged: u32,
    learned_hint: Option<u32>,
    last_written: Option<u32>,
    restore_done: bool,
    /// 已学技能数的持久化（改 cfg 里的基础上限/每技能后不用重新猜）
    state_path: Option<std::path::PathBuf>,
    saved_state: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct ParamRows {
    /// EquipParamGoods 1000 纸人 行首
    goods1000: Vec<usize>,
    /// EquipParamGoods 1001 临时(血)纸人 行首
    goods1001: Vec<usize>,
    /// EquipParamGoods 3800 纸人漂流 行首
    goods3800: Vec<usize>,
    /// ResourceItemParam goodsId=1000 行首（初始纸人 + 上限成长浮点）
    res1000: Vec<usize>,
}

/// 该地址所在内存页当前是否可写
fn region_writable(addr: usize) -> bool {
    use core::ffi::c_void;
    use windows::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, VirtualQuery};
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    let w = unsafe {
        VirtualQuery(
            Some(addr as *const c_void),
            &mut mbi,
            core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if w == 0 {
        return false;
    }
    let p = mbi.Protect.0;
    // PAGE_READWRITE 0x04 / WRITECOPY 0x08 / EXECUTE_READWRITE 0x40 / EXECUTE_WRITECOPY 0x80
    p & 0xCC != 0
}

/// 让整页可写（param 常被放在只读页里）
fn force_writable(addr: usize) -> bool {
    use core::ffi::c_void;
    use windows::Win32::System::Memory::{
        PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect,
    };
    let mut old = PAGE_PROTECTION_FLAGS(0);
    unsafe {
        if VirtualProtect(addr as *const c_void, 1, PAGE_READWRITE, &mut old).is_ok() {
            return true;
        }
        VirtualProtect(addr as *const c_void, 1, PAGE_EXECUTE_READWRITE, &mut old).is_ok()
    }
}

impl Emblem {
    pub fn new(cfg: impl AsRef<Path>) -> Emblem {
        let mut emblem = Emblem {
            enabled: true,
            base_cap: DEFAULT_BASE_CAP,
            per_skill: DEFAULT_PER_SKILL,
            legacy_base: DEFAULT_LEGACY_BASE,
            legacy_per: DEFAULT_LEGACY_PER,
            skill_count: DEFAULT_SKILL_COUNT,
            learned: -1,
            cap_offset: 0x146,
            cap_width: 2,
            learned_offset: 0,
            restore_offset: 0x170,
            restore_value: 25,
            drift: 9,
            initial_paper: 30,
            param_rows: None,
            param_scans: 0,
            param_patch_enabled: false,
            scan_pass: 0,
            scan_addr: 0,
            scan_end: 0,
            scan_off: 0,
            scan_ok: false,
            scan_done: false,
            scanned: 0,
            diag: [0; 4],
            probe: false,
            probe_cap: 0,
            probe_count: 0,
            window: DEFAULT_WINDOW,
            ticks: 0,
            reported: HashSet::new(),
            snapshot: Vec::new(),
            inventory_dumped: false,
            candidates_logged: 0,
            deltas_logged: 0,
            learned_hint: None,
            last_written: None,
            restore_done: false,
            state_path: None,
            saved_state: None,
        };
        let cfg_path = cfg.as_ref().to_path_buf();
        if let Ok(text) = fs::read_to_string(&cfg_path) {
            emblem.load(&text);
        }
        emblem.load_state(&cfg_path);
        log::warn!(
            "emblem settings: enabled={} base={} per_skill={} learned={} cap_off=0x{:x} width={} drift={} initial={} param_patch={} probe={}",
            emblem.enabled,
            emblem.base_cap,
            emblem.per_skill,
            emblem.learned,
            emblem.cap_offset,
            emblem.cap_width,
            emblem.drift,
            emblem.initial_paper,
            emblem.param_patch_enabled,
            emblem.probe
        );
        emblem
    }

    fn state_file(cfg: &Path) -> std::path::PathBuf {
        cfg.with_file_name("battle_instinct_emblem.state")
    }

    /// 读回上次记下的「已学上限技能数」（第一次没有就按上限反推）
    fn load_state(&mut self, cfg: &Path) {
        let path = Self::state_file(cfg);
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(v) = text.trim().parse::<u32>() {
                let v = v.min(self.skill_count);
                self.learned_hint = Some(v);
                self.saved_state = Some(v);
            }
        }
        self.state_path = Some(path);
    }

    /// 记住「已学上限技能数」：改 cfg 里的基础上限 / 每技能加成后不用重新猜
    fn save_state(&mut self) {
        let (Some(path), Some(k)) = (self.state_path.as_ref(), self.learned_hint) else {
            return;
        };
        if self.saved_state == Some(k) {
            return;
        }
        if fs::write(path, k.to_string()).is_ok() {
            self.saved_state = Some(k);
        }
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, value)) = split_kv(line) else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            match key.as_str() {
                "纸人上限功能" | "paper cap fix" => self.enabled = parse_bool(value),
                "纸人初始上限" | "纸人基础上限" | "paper initial cap" | "paper base cap" => {
                    if let Some(v) = parse_int(value) {
                        self.base_cap = v.max(0) as u32;
                    }
                }
                "技能增加上限" | "每个上限技能加成" | "per skill bonus" => {
                    if let Some(v) = parse_int(value) {
                        self.per_skill = v.max(0) as u32;
                    }
                }
                "旧基础上限" | "legacy base cap" => {
                    if let Some(v) = parse_int(value) {
                        self.legacy_base = v.max(0) as u32;
                    }
                }
                "旧每技能加成" | "legacy per skill" => {
                    if let Some(v) = parse_int(value) {
                        self.legacy_per = v.max(1) as u32;
                    }
                }
                "上限技能数" | "skill count" => {
                    if let Some(v) = parse_int(value) {
                        self.skill_count = v.max(1) as u32;
                    }
                }
                "已习得上限技能数" | "learned skills" => {
                    if let Some(v) = parse_int(value) {
                        self.learned = v as i32;
                    }
                }
                "纸人上限偏移" | "cap offset" => {
                    if let Some(v) = parse_int(value) {
                        self.cap_offset = v.max(0) as usize;
                    }
                }
                "纸人上限宽度" | "cap width" => {
                    if let Some(v) = parse_int(value) {
                        self.cap_width = match v {
                            1 | 2 | 4 => v as usize,
                            _ => 4,
                        };
                    }
                }
                "已学技能数偏移" | "learned offset" => {
                    if let Some(v) = parse_int(value) {
                        self.learned_offset = v.max(0) as usize;
                    }
                }
                "纸人还原偏移" | "restore offset" => {
                    if let Some(v) = parse_int(value) {
                        self.restore_offset = v.max(0) as usize;
                    }
                }
                "纸人还原值" | "restore value" => {
                    if let Some(v) = parse_int(value) {
                        self.restore_value = v.max(0) as u32;
                    }
                }
                "纸人漂流" | "纸人漂流上限" | "drift" | "drift cap" => {
                    if let Some(v) = parse_int(value) {
                        self.drift = v.max(0) as u32;
                    }
                }
                "初始纸人" | "initial paper" => {
                    if let Some(v) = parse_int(value) {
                        self.initial_paper = v.max(0) as u32;
                    }
                }
                "纸人内存参数" | "param memory patch" => {
                    self.param_patch_enabled = parse_bool(value);
                }
                "纸人探针" | "probe" => self.probe = parse_bool(value),
                "探针上限值" | "probe cap" => {
                    if let Some(v) = parse_int(value) {
                        self.probe_cap = v.max(0) as u32;
                    }
                }
                "探针数量值" | "probe count" => {
                    if let Some(v) = parse_int(value) {
                        self.probe_count = v.max(0) as u32;
                    }
                }
                "探针窗口" | "probe window" => {
                    if let Some(v) = parse_int(value) {
                        self.window = (v.max(0x100) as usize).min(0x100000);
                    }
                }
                _ => (),
            }
        }
    }

    pub fn tick(&mut self) {
        let Some(player) = player_data() else {
            return;
        };
        self.ticks += 1;

        if self.probe {
            if self.ticks % 60 == 0 {
                self.probe_scan(player);
                self.probe_deltas(player);
            }
            if !self.inventory_dumped && self.ticks > 240 {
                self.inventory_dumped = true;
                self.dump_inventory();
            }
        }

        if self.enabled && self.param_patch_enabled {
            // 增量扫描：每帧最多 1MB，找到即停，不会卡主线程
            if !self.scan_done {
                self.scan_step(0x10_0000);
            }
            if self.ticks % 300 == 1 {
                self.apply_params();
            }
        }

        if self.enabled && self.cap_offset > 0 {
            self.apply(player as *mut u8);
        }
    }

    /// 每帧增量扫描（最多 budget 字节），只找纸人相关 param 行 —— 绝不长时间阻塞主线程。
    /// 优先扫「≤64MB 的文件映射」（regulation 通常就在那），再扫私有堆。
    fn scan_step(&mut self, budget: usize) {
        use core::ffi::c_void;
        use windows::Win32::System::Memory::{
            MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
        };

        let mut budget = budget;
        while budget > 0 && !self.scan_done {
            if self.scan_addr == 0 {
                self.scan_addr = 0x1_0000;
            }
            if self.scan_addr >= self.scan_end {
                // 找下一个候选区域
                let mut mbi = MEMORY_BASIC_INFORMATION::default();
                let w = unsafe {
                    VirtualQuery(
                        Some(self.scan_addr as *const c_void),
                        &mut mbi,
                        core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                };
                if w == 0 {
                    self.finish_scan("address space walked");
                    return;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                let prot = mbi.Protect.0;
                let accessible = mbi.State.0 == MEM_COMMIT.0
                    && prot != 0
                    && (prot & PAGE_GUARD.0) == 0
                    && (prot & PAGE_NOACCESS.0) == 0;
                let mapped_small = mbi.Type.0 == 0x40000 && size <= 0x400_0000;
                let private = mbi.Type.0 == 0x20000;
                let want = match self.scan_pass {
                    0 => mapped_small,
                    _ => private,
                };
                self.scan_addr = base;
                self.scan_end = base + size.max(0x1000);
                self.scan_off = 0;
                self.scan_ok = accessible && want && size > 0 && size <= 0x4000_0000;
                if !self.scan_ok {
                    self.scan_addr = self.scan_end;
                    continue;
                }
            }
            let avail = self.scan_end.saturating_sub(self.scan_addr + self.scan_off);
            if avail < 8 {
                self.scan_addr = self.scan_end;
                continue;
            }
            let step = avail.min(budget).min(0x10_0000);
            let at = self.scan_addr + self.scan_off;
            let slice = unsafe { core::slice::from_raw_parts(at as *const u8, step) };
            self.match_slice(slice, at);
            self.scan_off += step;
            self.scanned += step;
            budget = budget.saturating_sub(step);
            if self.scanned >= SCAN_MAX_BYTES {
                self.finish_scan("budget reached");
            }
        }
    }

    fn finish_scan(&mut self, why: &str) {
        self.scan_done = true;
        if self.scan_pass == 0 {
            // 第一遍（文件映射）没找到就换私有堆再扫一遍
            self.scan_pass = 1;
            self.scan_addr = 0x1_0000;
            self.scan_end = 0;
            self.scan_off = 0;
            self.scan_done = false;
            log::warn!("emblem: param scan pass 1 (mapped) done ({why}); trying private heap");
            return;
        }
        let found = self.param_rows.as_ref().map(|r| {
            (r.goods1000.len(), r.goods1001.len(), r.goods3800.len(), r.res1000.len())
        });
        match found {
            Some((a, b, c, d)) if a + b + c + d > 0 => log::warn!(
                "emblem: param scan done ({why}, scanned 0x{:x}): 1000={a}x 1001={b}x 3800={c}x res={d}x",
                self.scanned
            ),
            _ => log::warn!(
                "emblem: param scan done ({why}, scanned 0x{:x}, diag 999={} 1021={} 3801={} 300050={}): nothing found",
                self.scanned,
                self.diag[0],
                self.diag[1],
                self.diag[2],
                self.diag[3]
            ),
        }
    }

    /// 在一段内存里找锚点（多重校验，避免误写）
    fn match_slice(&mut self, slice: &[u8], base: usize) {
        if self.param_rows.is_none() {
            self.param_rows = Some(ParamRows::default());
        }
        let mut i = 0usize;
        while i + 8 <= slice.len() {
            let v = u32::from_le_bytes([slice[i], slice[i + 1], slice[i + 2], slice[i + 3]]);
            let at = base + i;
            match v {
                0x3E7 if at >= 0x6C => {
                    self.diag[0] = self.diag[0].saturating_add(1);
                    let row = at - 0x6C;
                    let p = row as *const u8;
                    if read_u(p, 0x1C, 4) == 1020
                        && read_u(p, 0x2C, 4) == 1
                        && (1..=400).contains(&read_u(p, 0x36, 2))
                    {
                        if let Some(r) = self.param_rows.as_mut() {
                            if !r.goods1000.contains(&row) {
                                r.goods1000.push(row);
                            }
                        }
                    }
                }
                1021 if at >= 0x1C => {
                    self.diag[1] = self.diag[1].saturating_add(1);
                    let row = at - 0x1C;
                    let p = row as *const u8;
                    if read_u(p, 0x2C, 4) == 1 && (1..=400).contains(&read_u(p, 0x36, 2)) {
                        if let Some(r) = self.param_rows.as_mut() {
                            if !r.goods1001.contains(&row) {
                                r.goods1001.push(row);
                            }
                        }
                    }
                }
                0xED9 if at >= 0x18 => {
                    self.diag[2] = self.diag[2].saturating_add(1);
                    let row = at - 0x18;
                    let p = row as *const u8;
                    if read_u(p, 0x70, 4) == 1000 && read_u(p, 0x8C, 2) <= 999 {
                        if let Some(r) = self.param_rows.as_mut() {
                            if !r.goods3800.contains(&row) {
                                r.goods3800.push(row);
                            }
                        }
                    }
                }
                300050 => {
                    self.diag[3] = self.diag[3].saturating_add(1);
                    let p = at as *const u8;
                    if read_u(p, 0x0C, 4) == 1000 {
                        if let Some(r) = self.param_rows.as_mut() {
                            if !r.res1000.contains(&at) {
                                r.res1000.push(at);
                            }
                        }
                    }
                }
                _ => {}
            }
            i += 4;
        }
    }

    /// 把 cfg 里的值写进已定位的 param 行（廉价，可频繁调用）
    fn apply_params(&mut self) {
        let Some(rows) = self.param_rows.clone() else {
            return;
        };
        let cap = self.base_cap.min(999) as u16;
        let drift = self.drift.min(999) as u16;
        let initial = self.initial_paper.min(9999) as u32;
        let max_cap = (self.base_cap + self.per_skill * self.skill_count) as f32;

        for &row in &rows.goods1000 {
            let p = row as *const u8;
            if read_u(p, 0x1C, 4) != 1020 {
                continue;
            }
            if read_u(p, 0x36, 2) as u16 != cap && writable_row(row) {
                write_u(row as *mut u8, 0x36, 2, cap as u64);
                log::warn!("emblem: [param] 纸人上限 = {cap}");
            }
        }
        for &row in &rows.goods1001 {
            let p = row as *const u8;
            if read_u(p, 0x2C, 4) != 1 {
                continue;
            }
            if read_u(p, 0x36, 2) as u16 != drift && writable_row(row) {
                write_u(row as *mut u8, 0x36, 2, drift as u64);
                log::warn!("emblem: [param] 血纸人上限 = {drift}");
            }
        }
        for &row in &rows.goods3800 {
            let p = row as *const u8;
            if read_u(p, 0x70, 4) != 1000 {
                continue;
            }
            if read_u(p, 0x8C, 2) as u16 != drift && writable_row(row) {
                write_u(row as *mut u8, 0x8C, 2, drift as u64);
                log::warn!("emblem: [param] 纸人漂流给的数量 = {drift}");
            }
        }
        for &row in &rows.res1000 {
            let p = row as *const u8;
            if read_u(p, 0x0C, 4) != 1000 {
                continue;
            }
            if !writable_row(row) {
                continue;
            }
            if read_u(p, 0x10, 4) as u32 != initial {
                write_u(row as *mut u8, 0x10, 4, initial as u64);
                log::warn!("emblem: [param] 初始纸人 = {initial}");
            }
            if read_u(p, 0x14, 4) as u32 != (self.base_cap as f32 / 4.0).to_bits() {
                write_f32(row as *mut u8, 0x14, self.base_cap as f32 / 4.0);
            }
            if read_u(p, 0x18, 4) as u32 != (max_cap / 4.0).to_bits() {
                write_f32(row as *mut u8, 0x18, max_cap / 4.0);
            }
            if read_u(p, 0x20, 4) as u32 != max_cap.to_bits() {
                write_f32(row as *mut u8, 0x20, max_cap);
            }
        }
    }

    //------------------------------------------------------------------ 定位探针
    fn probe_scan(&mut self, player: *const u8) {
        for width in [1usize, 2, 4] {
            for off in (0..self.window.saturating_sub(width)).step_by(1) {
                let value = read_u(player, off, width);
                for (label, want) in [("上限", self.probe_cap), ("数量", self.probe_count)] {
                    if want != 0 && value == want as u64 && self.reported.insert(off * 8 + width) {
                        log::warn!(
                            "emblem probe: PlayerData+0x{off:x} (u{}) == HUD {label} {want}",
                            width * 8
                        );
                    }
                }
            }
        }
        // 不知道 HUD 数值时：把「像上限的小整数」列出来（去重、限量）
        if self.candidates_logged < 80 {
            for off in (0..self.window.saturating_sub(2)).step_by(2) {
                let v16 = read_u(player, off, 2);
                if (10..=130).contains(&v16) && self.reported.insert(0x2000_0000 + off) {
                    log::warn!("emblem probe: candidate PlayerData+0x{off:x} u16={v16}");
                    self.candidates_logged += 1;
                    if self.candidates_logged >= 80 {
                        break;
                    }
                }
            }
        }
    }

    fn probe_deltas(&mut self, player: *const u8) {
        let len = self.window.min(0x400);
        if self.snapshot.len() != len {
            self.snapshot = read_bytes(player, len);
            return;
        }
        if self.deltas_logged >= 160 {
            self.snapshot = read_bytes(player, len);
            return;
        }
        let now = read_bytes(player, len);
        let mut logged = 0;
        let mut off = 0;
        while off + 4 <= len && logged < 24 {
            let a = u32::from_le_bytes([
                self.snapshot[off],
                self.snapshot[off + 1],
                self.snapshot[off + 2],
                self.snapshot[off + 3],
            ]);
            let b = u32::from_le_bytes([now[off], now[off + 1], now[off + 2], now[off + 3]]);
            let changed = a != b
                && !(a == 0xffff_ffff && b == 0xffff_ffff)
                && (a.abs_diff(b) <= 4096 || b.abs_diff(a) <= 4096);
            if changed && self.reported.insert(0x1000_0000 + off) {
                log::warn!("emblem probe: PlayerData+0x{off:x} u32 {a} -> {b}");
                self.deltas_logged += 1;
                logged += 1;
            }
            off += 4;
        }
        self.snapshot = now;
    }

    fn dump_inventory(&self) {
        let Some(inventory) = inventory_ptr() else {
            log::warn!("emblem probe: inventory pointer is null");
            return;
        };
        let head = read_bytes(inventory, 0x100);
        let words: Vec<String> = head
            .chunks_exact(4)
            .take(32)
            .enumerate()
            .map(|(i, c)| format!("{:02x}:{:08x}", i, u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
            .collect();
        log::warn!("emblem probe: inventory head {}", words.join(" "));

        for off in (0..INVENTORY_SCAN_LEN.saturating_sub(8)).step_by(4) {
            let id = read_u(inventory, off, 4) as u32;
            if WATCH_ITEM_IDS.contains(&id) {
                let next = read_u(inventory, off + 4, 4) as u32;
                let prev = if off >= 4 { read_u(inventory, off - 4, 4) as u32 } else { 0 };
                log::warn!(
                    "emblem probe: inventory+0x{off:x} id={id} prev={prev} next={next}"
                );
            }
        }
    }

    //------------------------------------------------------------------ 修正
    fn apply(&mut self, player: *mut u8) {
        let cur = read_u(player, self.cap_offset, self.cap_width) as u32;

        // 保护：读到的值不像「纸人上限」就不写（多半是偏移/宽度填错）
        if !(5..=200).contains(&cur) {
            if self.reported.insert(0x3000_0000) {
                log::warn!(
                    "emblem: cap offset 0x{:x} (u{}) reads {} - not cap-like, skip writing",
                    self.cap_offset,
                    self.cap_width * 8,
                    cur
                );
            }
            return;
        }

        if let (Some(last), true) = (self.last_written, cur > 0) {
            // 游戏在我们的写入值上又加了 1~4：说明刚学会了一个「上限技能」
            if cur > last && cur - last <= 4 {
                let hint = self.learned_hint.unwrap_or(0) + 1;
                self.learned_hint = Some(hint);
                log::warn!("emblem: skill learned (cap {last} -> {cur}), count={hint}");
            }
        }

        let learned = if self.learned >= 0 {
            self.learned as u32
        } else if self.learned_offset > 0 {
            (read_u(player, self.learned_offset, 4) as u32).min(self.skill_count)
        } else if let Some(hint) = self.learned_hint {
            hint
        } else {
            infer_learned(
                cur,
                self.base_cap,
                self.per_skill,
                self.legacy_base,
                self.legacy_per,
                self.skill_count,
            )
        };
        self.learned_hint = Some(learned);
        self.save_state();

        let target = self.base_cap + self.per_skill * learned.min(self.skill_count);
        if cur != target {
            write_u(player, self.cap_offset, self.cap_width, target as u64);
            self.last_written = Some(target);
            log::warn!("emblem: cap {cur} -> {target} (learned={learned})");
        }

        if self.restore_offset > 0 && !self.restore_done {
            let now = read_u(player, self.restore_offset, 2) as u32;
            if now != self.restore_value {
                write_u(player, self.restore_offset, 2, self.restore_value as u64);
                log::warn!(
                    "emblem: restore offset 0x{:x}: {} -> {}",
                    self.restore_offset,
                    now,
                    self.restore_value
                );
            }
            self.restore_done = true;
    }
}
}

/// 从当前上限反推已学技能数：
/// 30+10k（本模块写过的）优先，否则按老存档 15+2k 推。
fn infer_learned(
    cur: u32,
    base: u32,
    per: u32,
    legacy_base: u32,
    legacy_per: u32,
    count: u32,
) -> u32 {
    if cur >= base && per > 0 {
        let k = (cur - base + per / 2) / per;
        if k <= count {
            return k;
        }
    }
    if cur > legacy_base && legacy_per > 0 {
        let k = (cur - legacy_base + legacy_per / 2) / legacy_per;
        return k.min(count);
    }
    0
}

//----------------------------------------------------------------------------
//
//  Raw memory helpers（全部做窗口/指针校验，宁可不动也不越界）
//
//----------------------------------------------------------------------------

fn player_data() -> Option<*const u8> {
    unsafe {
        let game_data = game::game_data();
        if game_data.is_null() {
            return None;
        }
        let player = (*game_data).player_data;
        if player.is_null() {
            return None;
        }
        Some(player as *const u8)
    }
}

fn inventory_ptr() -> Option<*const u8> {
    unsafe {
        let player = player_data()? as *const game::PlayerData;
        let inventory = (*player).inventory_data;
        if inventory.is_null() {
            return None;
        }
        Some((inventory as *const u8).add(0x10))
    }
}

fn read_u(base: *const u8, offset: usize, width: usize) -> u64 {
    unsafe {
        let p = base.add(offset);
        match width {
            1 => (p as *const u8).read_unaligned() as u64,
            2 => u16::from_le((p as *const u16).read_unaligned()) as u64,
            _ => u32::from_le((p as *const u32).read_unaligned()) as u64,
        }
    }
}

fn write_u(base: *mut u8, offset: usize, width: usize, value: u64) {
    unsafe {
        let p = base.add(offset);
        match width {
            1 => (p as *mut u8).write_unaligned(value as u8),
            2 => (p as *mut u16).write_unaligned((value as u16).to_le()),
            _ => (p as *mut u32).write_unaligned((value as u32).to_le()),
        }
    }
}

fn write_f32(base: *mut u8, offset: usize, value: f32) {
    unsafe {
        ((base.add(offset)) as *mut u32).write_unaligned(value.to_bits());
    }
}

fn writable_row(addr: usize) -> bool {
    region_writable(addr) || force_writable(addr)
}

fn read_bytes(base: *const u8, len: usize) -> Vec<u8> {
    unsafe { std::slice::from_raw_parts(base, len).to_vec() }
}

fn relative_to(player: *const u8, addr: usize) -> String {
    let p = player as usize;
    if addr >= p && addr - p <= 0x10_0000 {
        format!(" (PlayerData+0x{:x})", addr - p)
    } else {
        String::new()
    }
}

fn neighbors(slice: &[u8], at: usize) -> String {
    let mut out = Vec::new();
    let start = at.saturating_sub(6) & !1;
    let end = (at + 10).min(slice.len());
    let mut i = start;
    while i + 1 < end {
        out.push(format!("{:?}", u16::from_le_bytes([slice[i], slice[i + 1]])));
        i += 2;
    }
    out.join(",")
}
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let body = line.trim().strip_prefix('#')?.trim();
    for sep in [':', '：'] {
        if let Some((k, v)) = body.split_once(sep) {
            return Some((k, v));
        }
    }
    None
}

fn parse_int(raw: &str) -> Option<i64> {
    let text = raw
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if text.is_empty() {
        return None;
    }
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => text.parse::<i64>().ok(),
    }
}

fn parse_bool(raw: &str) -> bool {
    let v = raw.trim().to_ascii_lowercase();
    matches!(v.as_str(), "1" | "开" | "on" | "true" | "yes" | "是") || v.starts_with("开")
}
