@echo off
setlocal
chcp 936 >nul
title 只狼 · 纸血挂
set "HERE=%~dp0"
set "PAPERDOLL_CFG=%HERE%battle_instinct.cfg"
set "PAPERDOLL_LANG=zh"
if /i "%~1"=="revert" set "PAPERDOLL_REVERT=1"
if /i "%~1"=="rollback" set "PAPERDOLL_REVERT=1"
echo ==========================================================
echo   只狼 · 纸血挂
echo ----------------------------------------------------------
echo   读取同目录 battle_instinct.cfg 里的这几行：
echo     纸人上限功能: 开 / 关
echo     纸人初始上限: 25     技能增加上限: 5     纸人漂流: 9
echo     血量功能: 开 / 关     血量倍率: 3
echo   「纸人上限功能: 关」/「血量功能: 关」= 跳过对应部分。
echo   改完 cfg 双击本文件；回滚：纸血挂.bat revert
echo ==========================================================
echo.
if not exist "%PAPERDOLL_CFG%" (
  echo [错误] 同目录找不到 battle_instinct.cfg，请把它和本文件放在一起。
  goto done
)
set "TMPPS=%TEMP%\zhiren_%RANDOM%%RANDOM%.ps1"
powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -LiteralPath '%~f0' -Encoding Default | Select-Object -Skip 43 | Set-Content -LiteralPath '%TMPPS%' -Encoding Default"
if not exist "%TMPPS%" (
  echo [错误] 解出临时脚本失败。
  goto done
)
powershell -NoProfile -ExecutionPolicy Bypass -File "%TMPPS%" -Apply
set "RC=%ERRORLEVEL%"
del "%TMPPS%" >nul 2>nul
if not "%RC%"=="0" echo [提示] 脚本退出码 %RC%
goto done
:done
echo.
echo 完成。按任意键退出。
pause >nul
endlocal
exit /b

rem ============ 以下为内嵌 PowerShell 代码（自解执行；改 tools/patch_paper_params.ps1 后跑 tools/make_bat.ps1 重新生成）============
<#
  patch_paper_params.ps1 - paper doll + player max HP param one-shot patcher
  ------------------------------------------------------------------------
  纸人（Spirit Emblem / 形代）相关 param 一键修改。
  不改 DLL、不覆盖别人的 mod：直接对 Mod Engine 加载的 regulation 参数包
  mods\param\gameparam\gameparam.parambnd.dcx 做字段级补丁（自动备份、可回滚）。

  字段来源（本机两套 mod 包对比 + 逐行 dump 实测）：
    EquipParamGoods  id=1000  纸人      row+0x36 (u16) = 纸人上限         vanilla 15
    EquipParamGoods  id=1001  临时纸人  row+0x36 (u16) = 临时纸人上限     vanilla 5
    EquipParamGoods  id=3800  纸人漂流  row+0x8C (u16) = 一次给多少临时   vanilla 5
    ResourceItemParam goodsId=1000 行  +0x10 (u32)    = 开局携带纸人数    vanilla 6
    ResourceItemParam goodsId=1000 行  +0x14/+0x18/+0x1C/+0x20 (f32)
        = 基础上限/4、满上限/4、技能个数、满上限（推断，用于「每个技能 +N」）

  玩家最大 HP（k=2，血条适配）：
    CalcCorrectGraph row 500  stageMaxGrowVal0 = 320*k, stageMaxGrowVal1..4 = 1120*k
    MenuParam row 0           PlayerMaxHpLimit = 1920*k, PlayerQuarterHp = 20*k,
                              HealthHp* 同步 *k，PlayerMaxAddHp = 0
    默认 k=2：开局最大 HP 640，每条念珠串 +160，10 条后 2240；满级血条长度保持原版比例
    cfg: 血量功能: 开/关  血量倍率: 3   /   -SkipHp  -HpMultiplier 3

  输出语言：
    -Lang en / -Lang zh，或环境变量 PAPERDOLL_LANG=en|zh（发布包 .bat 用它切换）
    缺省按系统区域：中文系统 = zh，其余 = en

  用法：
      只看不改:  powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1
      应用:      powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Apply
      回滚:      powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Revert
      自定义:    ... -Apply -InitialPaper 30 -BaseCap 30 -PerSkillBonus 10 -TempCap 15 -DriftAmount 15
      血量:      ... -Apply -HpMultiplier 2     （默认已开启；关：-SkipHp 或 cfg「血量功能: 关」）

  说明：输出串全为 ASCII，中文一律写成 \uXXXX 转义；正文里剩下的中文只有 cfg 键名，
  两者都能被 GBK 编码，所以嵌进 .bat 尾部再被 `Get-Content -Encoding Default` 读出来也不会乱码。
#>
[CmdletBinding()]
param(
    [string]$ParamFile  = '',
    [string]$CfgFile    = $env:PAPERDOLL_CFG,
    [bool]  $UseCfg      = $true,
    [int]$InitialPaper  = 30,
    [int]$BaseCap       = 30,
    [int]$PerSkillBonus = 10,
    [int]$SkillCount    = 5,
    [int]$TempCap       = 15,
    [int]$DriftAmount   = 15,
    [double]$HpMultiplier = 2.0,
    [switch]$SkipHp,
    [string]$Lang       = $(if ($env:PAPERDOLL_LANG) { $env:PAPERDOLL_LANG } else { 'auto' }),
    [switch]$SkipGrowthFloats,
    [switch]$Apply,
    [switch]$Force,
    [switch]$Revert
)

$ErrorActionPreference = 'Stop'

# 发布包 .bat 用环境变量传「回滚 / 强制」：脚本被 -File 调用时拿不到命令行开关，这里补上
function Test-EnvFlag([string]$name) {
    $v = [Environment]::GetEnvironmentVariable($name)
    return ($v -and ($v.Trim() -match '^(1|true|yes|on)$'))
}
if (-not $Revert -and (Test-EnvFlag 'PAPERDOLL_REVERT')) { $Revert = $true }
if (-not $Force  -and (Test-EnvFlag 'PAPERDOLL_FORCE'))  { $Force  = $true }

if ($Lang -notin @('en', 'zh')) {
    $Lang = if ((Get-Culture).TwoLetterISOLanguageName -eq 'zh') { 'zh' } else { 'en' }
}

# ---- 输出串（中文一律用 \uXXXX 转义，保证嵌进 GBK 的 .bat 也不会乱码）----
$S = @{
    en = @{
        off = 'cfg has "paper cap fix: off" - skipping param writes (use -Force to override, -Revert to roll back)'
        noGameDir = 'Sekiro folder not found; use -ParamFile to point at gameparam.parambnd.dcx'
        doll = 'Paper doll'
        temp = 'Temporary paper doll'
        drift = 'Paper doll drift'
        initial = 'Starting paper dolls'
        growthF1 = 'Cap growth f1 (base cap / 4)'
        growthF2 = 'Cap growth f2 (max cap / 4)'
        growthF3 = 'Cap growth f3 (cap-skill count)'
        growthF4 = 'Cap growth f4 (max cap)'
        target = 'Target file'
        plan = 'Planned changes:'
        missing1000 = '  !! EquipParamGoods has no id=1000'
        missing1001 = '  !! EquipParamGoods has no id=1001'
        missing3800 = '  !! EquipParamGoods has no id=3800 (paper doll drift)'
        missingRes = '  !! ResourceItemParam has no goodsId=1000 row'
        floats = '  (the 4 floats below are the only param-side entry for "per skill bonus"; inferred fields, -SkipGrowthFloats disables them)'
        dry = '(dry run: nothing written; add -Apply to actually write)'
        ok = 'Written: {0}  (backup {1})'
        verify = 'Verify: cap={0} temp cap={1} drift gives={2} starting dolls={3} max cap={4}'
        noParam = 'param file not found: {0}'
        noBackup = 'no backup found ({0}.bak-*), nothing to roll back to'
        rolledBack = 'Rolled back: {0} -> {1}'
        noGoods = 'EquipParamGoods.param not found in the param package'
        noRes = 'ResourceItemParam.param not found in the param package'
        missingCcg = 'CalcCorrectGraph.param not found in the param package'
        missingMenu = 'MenuParam.param not found in the param package'
        missingCcgRow = 'CalcCorrectGraph row 500 not found'
        missingMenuRow = 'MenuParam row 0 not found'
        verifyHp = 'Verify HP: start={0} max={1} limit={2} quarter={3}'
    }
    zh = @{
        off = '当前 cfg 里「纸人上限功能: 关」→ 跳过 param 写入（想强制加 -Force，想回滚加 -Revert）'
        noGameDir = '找不到 Sekiro 目录，请用 -ParamFile 指定 gameparam.parambnd.dcx'
        doll = '纸人'
        temp = '临时纸人'
        drift = '纸人漂流'
        initial = '初始纸人'
        growthF1 = '上限成长 f1（基础上限 / 4）'
        growthF2 = '上限成长 f2（满上限 / 4）'
        growthF3 = '上限成长 f3（上限技能个数）'
        growthF4 = '上限成长 f4（满上限）'
        target = '目标文件'
        plan = '计划修改:'
        missing1000 = '  !! EquipParamGoods 里没有 id=1000'
        missing1001 = '  !! EquipParamGoods 里没有 id=1001'
        missing3800 = '  !! EquipParamGoods 里没有 id=3800（纸人漂流）'
        missingRes = '  !! ResourceItemParam 里没有 goodsId=1000 的行'
        floats = '  （以下 4 个浮点是「每个技能 +N」的唯一 param 侧入口，属推断字段，-SkipGrowthFloats 可关）'
        dry = '（演练模式：没有写入任何文件；加 -Apply 才真正写入）'
        ok = '已写入: {0}  （备份 {1}）'
        verify = '校验: 纸人上限={0} 临时上限={1} 漂流给={2} 初始纸人={3} 满上限={4}'
        noParam = '找不到参数文件: {0}'
        noBackup = '没有找到备份 ({0}.bak-*)，无法回滚'
        rolledBack = '已回滚: {0} -> {1}'
        noGoods = '参数包里没有 EquipParamGoods.param'
        noRes = '参数包里没有 ResourceItemParam.param'
        missingCcg = '参数包里没有 CalcCorrectGraph.param'
        missingMenu = '参数包里没有 MenuParam.param'
        missingCcgRow = 'CalcCorrectGraph 里没有 500 行'
        missingMenuRow = 'MenuParam 里没有 0 行'
        verifyHp = '校验 HP: 开局={0} 满级={1} 血条基准={2} 四分之一={3}'
    }
}
$T = $S[$Lang]

# ---- 从 battle_instinct.cfg 读三个可调项（cfg 优先，缺省用上面的默认值）----
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

function Find-GameDir {
    $cands = @(
        $env:PAPERDOLL_GAME_DIR,   # 显式指定游戏目录（可选，优先级最高）
        $scriptDir,
        'D:\game\steam\steamapps\common\Sekiro',
        'C:\Program Files (x86)\Steam\steamapps\common\Sekiro',
        'D:\Steam\steamapps\common\Sekiro',
        'E:\Steam\steamapps\common\Sekiro'
    )
    foreach ($k in @('HKLM:\SOFTWARE\WOW6432Node\Valve\Steam', 'HKLM:\SOFTWARE\Valve\Steam')) {
        try {
            $sp = (Get-ItemProperty -Path $k -ErrorAction Stop).InstallPath
            if ($sp) { $cands += (Join-Path $sp 'steamapps\common\Sekiro') }
        } catch { }
    }
    foreach ($c in $cands) { if ($c -and (Test-Path (Join-Path $c 'sekiro.exe'))) { return $c } }
    return ''
}

if (-not $CfgFile) {
    $local = Join-Path $scriptDir 'battle_instinct.cfg'
    if (Test-Path $local) { $CfgFile = $local } else { $CfgFile = '' }
}
$gameDir = Find-GameDir
if (-not $ParamFile) {
    if (-not $gameDir) { throw $T.noGameDir }
    $ParamFile = Join-Path $gameDir 'mods\param\gameparam\gameparam.parambnd.dcx'
    if (-not $CfgFile) { $CfgFile = Join-Path $gameDir 'battle_instinct.cfg' }
}
$driftTouched = $false
$keyMap = @{
  '纸人上限功能'            = 'CapFixOn'
  'paper cap fix'           = 'CapFixOn'
  '初始纸人'                = 'InitialPaper'
  '纸人初始上限'            = 'BaseCap'
  '纸人基础上限'            = 'BaseCap'
  '技能增加上限'            = 'PerSkillBonus'
  '每个上限技能加成'        = 'PerSkillBonus'
  '上限技能数'              = 'SkillCount'
  '纸人漂流'                = 'TempCap'      # 血纸人(临时)上限，同时作为一次给的数量
  '纸人漂流上限'            = 'TempCap'
  '纸人漂流数量'            = 'DriftAmount'  # 可选：单独指定一次给多少
  'initial paper'           = 'InitialPaper'
  'paper initial cap'       = 'BaseCap'
  'paper base cap'          = 'BaseCap'
  'per skill bonus'         = 'PerSkillBonus'
  'skill count'             = 'SkillCount'
  'drift'                   = 'TempCap'
  'drift cap'               = 'TempCap'
  'drift amount'            = 'DriftAmount'
}
if ($UseCfg -and (Test-Path $CfgFile)) {
  foreach ($line in (Get-Content $CfgFile -Encoding UTF8)) {
    $body = $line.TrimStart('#', ' ').Trim()
    foreach ($k in $keyMap.Keys) {
      if ($body.StartsWith($k + ':')) {
        $num = ($body.Substring($k.Length + 1).Trim() -replace '^([-\d]+).*$', '$1')
        if ($num -match '^-?\d+$') {
          $v = [int]$num
          switch ($keyMap[$k]) {
            'CapFixOn'      { $capFixOn = $v }
            'InitialPaper'  { $InitialPaper  = $v }
            'BaseCap'       { $BaseCap       = $v }
            'PerSkillBonus' { $PerSkillBonus = $v }
            'SkillCount'    { $SkillCount    = $v }
            'TempCap'       { $TempCap = $v; if (-not $driftTouched) { $DriftAmount = $v } }
            'DriftAmount'   { $DriftAmount = $v; $driftTouched = $true }
          }
        }
      }
    }
  }
}

# 「纸人上限功能: 开/关」是文字开关，单独读一次（关 = 不写 param）
$capFixOn = 1
if ($CfgFile -and (Test-Path $CfgFile)) {
    $swLine = Select-String -Path $CfgFile -Pattern '(纸人上限功能|paper cap fix)\s*[:：]' -Encoding UTF8 | Select-Object -First 1
    if ($swLine -and ($swLine.Line -match '[:：]\s*(关|off|false|no)')) { $capFixOn = 0 }
}
# 「血量功能: 开/关」以及倍率
$hpFixOn = 1
if ($CfgFile -and (Test-Path $CfgFile)) {
    foreach ($line in (Get-Content $CfgFile -Encoding UTF8)) {
        $body = $line.TrimStart('#', ' ').Trim()
        if ($body -match '^(血量功能|health fix)\s*[:：]\s*(.*)$') {
            if ($Matches[2] -match '^(关|off|false|no|0)\b') { $hpFixOn = 0 } else { $hpFixOn = 1 }
        }
        if ($body -match '^(血量倍率|hp multiplier|health multiplier)\s*[:：]\s*([0-9]+(?:\.[0-9]+)?)') {
            $HpMultiplier = [double]$Matches[2]
        }
    }
}
if ($HpMultiplier -le 0) { $HpMultiplier = 3.0 }
$doPaper = $Force -or ($capFixOn -ne 0)
$doHp = (-not $SkipHp) -and ($Force -or ($hpFixOn -ne 0))
if (-not $Revert -and -not $doPaper -and -not $doHp) {
    Write-Host $T.off
    return
}


$cs = @"
using System;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Collections.Generic;

public static class PaperParams
{
    public static uint ReadBE(byte[] b, int o)
    { return ((uint)b[o] << 24) | ((uint)b[o+1] << 16) | ((uint)b[o+2] << 8) | b[o+3]; }

    public static void WriteBE(byte[] b, int o, uint v)
    { b[o]=(byte)(v>>24); b[o+1]=(byte)(v>>16); b[o+2]=(byte)(v>>8); b[o+3]=(byte)v; }

    public static byte[] Expand(byte[] file)
    {
        int dcs = (int)ReadBE(file, 0x08);
        int dca = (int)ReadBE(file, 0x10);
        int unc = (int)ReadBE(file, dcs + 4);
        int cmp = (int)ReadBE(file, dcs + 8);
        string type = Encoding.ASCII.GetString(file, dcs + 16, 4);
        if (type != "DFLT") throw new Exception("unsupported DCX compression: " + type);
        byte[] sub = new byte[cmp];
        Array.Copy(file, dca + 8, sub, 0, cmp);
        using (var ms = new MemoryStream(sub))
        {
            ms.Position = 2;
            using (var ds = new DeflateStream(ms, CompressionMode.Decompress))
            {
                byte[] raw = new byte[unc];
                int read = 0, n;
                while (read < unc && (n = ds.Read(raw, read, unc - read)) > 0) read += n;
                return raw;
            }
        }
    }

    public static byte[] Zlib(byte[] raw)
    {
        byte[] deflated;
        using (var ms = new MemoryStream())
        {
            using (var ds = new DeflateStream(ms, CompressionLevel.Optimal, true)) { ds.Write(raw, 0, raw.Length); }
            deflated = ms.ToArray();
        }
        uint a = 1, b = 0;
        foreach (byte v in raw) { a = (a + v) % 65521; b = (b + a) % 65521; }
        uint adler = (b << 16) | a;
        byte[] outp = new byte[2 + deflated.Length + 4];
        outp[0] = 0x78; outp[1] = 0xDA;
        Array.Copy(deflated, 0, outp, 2, deflated.Length);
        WriteBE(outp, outp.Length - 4, adler);
        return outp;
    }

    public static byte[] Pack(byte[] template, byte[] bnd)
    {
        int dcs = (int)ReadBE(template, 0x08);
        int payloadStart = (int)ReadBE(template, 0x10) + 8;
        byte[] z = Zlib(bnd);
        byte[] outp = new byte[payloadStart + z.Length];
        Array.Copy(template, 0, outp, 0, payloadStart);
        Array.Copy(z, 0, outp, payloadStart, z.Length);
        WriteBE(outp, dcs + 4, (uint)bnd.Length);
        WriteBE(outp, dcs + 8, (uint)z.Length);
        return outp;
    }

    public class Entry { public string Name; public int Start; public int Size; }

    public static List<Entry> Entries(byte[] b)
    {
        var list = new List<Entry>();
        int count = (int)BitConverter.ToUInt32(b, 0x0C);
        int table = (int)BitConverter.ToUInt64(b, 0x10);
        int esize = (int)BitConverter.ToUInt64(b, 0x20);
        for (int k = 0; k < count; k++)
        {
            int o = table + k * esize;
            long size = (long)BitConverter.ToUInt64(b, o + 8);
            int start = (int)BitConverter.ToUInt32(b, o + 24);
            int n = (int)BitConverter.ToUInt32(b, o + 32);
            int e = n;
            while (e < b.Length - 1 && !(b[e] == 0 && b[e + 1] == 0)) e += 2;
            string name = Encoding.Unicode.GetString(b, n, e - n);
            name = name.Substring(name.LastIndexOf('\\') + 1);
            list.Add(new Entry { Name = name, Start = start, Size = (int)size });
        }
        return list;
    }

    public static Entry Find(List<Entry> es, string name)
    {
        foreach (var e in es) if (e.Name == name) return e;
        return null;
    }

    public static Dictionary<int,int> RowOffsets(byte[] b, int start, int size)
    {
        var map = new Dictionary<int,int>();
        int o = start + 0x40, end = start + size, prev = -1;
        while (o + 24 <= end)
        {
            int id = BitConverter.ToInt32(b, o);
            int off = BitConverter.ToInt32(b, o + 8);
            if (id < 0 || id > 400000) break;
            if (off <= 0 || off >= size) break;
            if (off <= prev) break;
            map[id] = start + off;
            prev = off;
            o += 24;
        }
        return map;
    }

    public static int FindRowByGoods(byte[] b, int start, int size, int goodsId)
    {
        for (int off = 0; off + 52 <= size; off += 52)
            if (BitConverter.ToUInt32(b, start + off + 0x0C) == (uint)goodsId) return start + off;
        return -1;
    }

    public static uint U32(byte[] b, int o) { return BitConverter.ToUInt32(b, o); }
    public static int I32(byte[] b, int o) { return BitConverter.ToInt32(b, o); }
    public static ushort U16(byte[] b, int o) { return BitConverter.ToUInt16(b, o); }
    public static float F32(byte[] b, int o) { return BitConverter.ToSingle(b, o); }
    public static void W16(byte[] b, int o, int v) { b[o]=(byte)v; b[o+1]=(byte)(v>>8); }
    public static void W32(byte[] b, int o, uint v) { BitConverter.GetBytes(v).CopyTo(b, o); }
    public static void WF32(byte[] b, int o, float v) { BitConverter.GetBytes(v).CopyTo(b, o); }
}
"@
Add-Type -TypeDefinition $cs -ErrorAction Stop

# ------------------------------------------------------------------ main logic
if (-not (Test-Path $ParamFile)) { throw ($T.noParam -f $ParamFile) }

if ($Revert) {
    $baks = Get-ChildItem ($ParamFile + '.bak-*') -ErrorAction SilentlyContinue | Sort-Object Name -Descending
    if (-not $baks) { throw ($T.noBackup -f $ParamFile) }
    $src = $baks[0].FullName
    Copy-Item $src $ParamFile -Force
    Write-Host ($T.rolledBack -f $src, $ParamFile)
    return
}

$orig = [System.IO.File]::ReadAllBytes($ParamFile)
$bnd  = [PaperParams]::Expand($orig)
$entries = [PaperParams]::Entries($bnd)

$gRows = @{}
$resRow = -1
$ccgRows = @{}
$menuRows = @{}
$maxCap = $BaseCap + $PerSkillBonus * $SkillCount

function Set-U16([int]$row, [int]$off, [int]$val, [string]$label) {
    $old = [PaperParams]::U16($bnd, $row + $off)
    [PaperParams]::W16($bnd, $row + $off, $val)
    Write-Host ("  {0,-46} {1} -> {2}" -f $label, $old, $val)
}
function Set-U32([int]$row, [int]$off, [int]$val, [string]$label) {
    $old = [PaperParams]::U32($bnd, $row + $off)
    [PaperParams]::W32($bnd, $row + $off, [uint32]$val)
    Write-Host ("  {0,-46} {1} -> {2}" -f $label, $old, $val)
}
function Set-F32([int]$row, [int]$off, [single]$val, [string]$label) {
    $old = [PaperParams]::F32($bnd, $row + $off)
    [PaperParams]::WF32($bnd, $row + $off, $val)
    Write-Host ("  {0,-46} {1} -> {2}" -f $label, $old, $val)
}

Write-Host ("{0}: {1}" -f $T.target, $ParamFile)
Write-Host $T.plan
if ($doPaper) {
    $goods = [PaperParams]::Find($entries, 'EquipParamGoods.param')
    $res   = [PaperParams]::Find($entries, 'ResourceItemParam.param')
    if (-not $goods) { throw $T.noGoods }
    if (-not $res)   { throw $T.noRes }
    $gRows = [PaperParams]::RowOffsets($bnd, $goods.Start, $goods.Size)

    if ($gRows.ContainsKey(1000)) {
        Set-U16 $gRows[1000] 0x36 $BaseCap ("EquipParamGoods 1000 " + $T.doll + " cap")
    } else { Write-Host $T.missing1000 }
    if ($gRows.ContainsKey(1001)) {
        Set-U16 $gRows[1001] 0x36 $TempCap ("EquipParamGoods 1001 " + $T.temp + " cap")
    } else { Write-Host $T.missing1001 }
    if ($gRows.ContainsKey(3800)) {
        Set-U16 $gRows[3800] 0x8C $DriftAmount ("EquipParamGoods 3800 " + $T.drift + " amount")
    } else { Write-Host $T.missing3800 }

    $resRow = [PaperParams]::FindRowByGoods($bnd, $res.Start, $res.Size, 1000)
    if ($resRow -ge 0) {
        Set-U32 $resRow 0x10 $InitialPaper ("ResourceItemParam 1000 " + $T.initial)
        if (-not $SkipGrowthFloats) {
            Write-Host $T.floats
            Set-F32 $resRow 0x14 ([single]($BaseCap / 4.0))   ("ResourceItemParam 1000 " + $T.growthF1)
            Set-F32 $resRow 0x18 ([single]($maxCap / 4.0))    ("ResourceItemParam 1000 " + $T.growthF2)
            Set-F32 $resRow 0x1C ([single]$SkillCount)        ("ResourceItemParam 1000 " + $T.growthF3)
            Set-F32 $resRow 0x20 ([single]$maxCap)            ("ResourceItemParam 1000 " + $T.growthF4)
        }
    } else { Write-Host $T.missingRes }
}

if ($doHp) {
    $ccg = [PaperParams]::Find($entries, 'CalcCorrectGraph.param')
    $menu = [PaperParams]::Find($entries, 'MenuParam.param')
    if (-not $ccg) { throw $T.missingCcg }
    if (-not $menu) { throw $T.missingMenu }
    $ccgRows = [PaperParams]::RowOffsets($bnd, $ccg.Start, $ccg.Size)
    $menuRows = [PaperParams]::RowOffsets($bnd, $menu.Start, $menu.Size)
    if (-not $ccgRows.ContainsKey(500)) { throw $T.missingCcgRow }
    if (-not $menuRows.ContainsKey(0)) { throw $T.missingMenuRow }

    $hpStart = [single](320.0 * $HpMultiplier)
    $hpMax = [single](1120.0 * $HpMultiplier)
    $hpLimit = [int][math]::Round(1920.0 * $HpMultiplier)
    $hpQuarter = [int][math]::Round(20.0 * $HpMultiplier)
    $hpNoDamage = [int][math]::Round(300.0 * $HpMultiplier)
    $hpLight = [int][math]::Round(250.0 * $HpMultiplier)
    $hpHeavy = [int][math]::Round(100.0 * $HpMultiplier)
    $hpDying = [int][math]::Round(1.0 * $HpMultiplier)

    Write-Host ("  HP max x {0}" -f $HpMultiplier)
    Set-F32 $ccgRows[500] 0x14 $hpStart "CalcCorrectGraph 500 stageMaxGrowVal0"
    Set-F32 $ccgRows[500] 0x18 $hpMax "CalcCorrectGraph 500 stageMaxGrowVal1"
    Set-F32 $ccgRows[500] 0x1C $hpMax "CalcCorrectGraph 500 stageMaxGrowVal2"
    Set-F32 $ccgRows[500] 0x20 $hpMax "CalcCorrectGraph 500 stageMaxGrowVal3"
    Set-F32 $ccgRows[500] 0x24 $hpMax "CalcCorrectGraph 500 stageMaxGrowVal4"
    Set-U32 $menuRows[0] 0x08 $hpLimit "MenuParam 0 PlayerMaxHpLimit"
    Set-U32 $menuRows[0] 0x1C 0 "MenuParam 0 PlayerMaxAddHp"
    Set-U32 $menuRows[0] 0x9C $hpQuarter "MenuParam 0 PlayerQuarterHp"
    Set-U32 $menuRows[0] 0x38 $hpNoDamage "MenuParam 0 HealthHpNoDamage"
    Set-U32 $menuRows[0] 0x3C $hpLight "MenuParam 0 HealthHpLightDamage"
    Set-U32 $menuRows[0] 0x40 $hpHeavy "MenuParam 0 HealthHpHeavyDamage"
    Set-U32 $menuRows[0] 0x44 $hpDying "MenuParam 0 HealthHpDying"
    Set-U32 $menuRows[0] 0x48 0 "MenuParam 0 HealthHpDead"
}

if (-not $Apply) {
    Write-Host ''
    Write-Host $T.dry
    return
}

$bak = $ParamFile + '.bak-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
Copy-Item $ParamFile $bak -Force
$packed = [PaperParams]::Pack($orig, $bnd)
[System.IO.File]::WriteAllBytes($ParamFile, $packed)

# verify round trip
$check = [PaperParams]::Expand([System.IO.File]::ReadAllBytes($ParamFile))
$ce = [PaperParams]::Entries($check)
Write-Host ''
Write-Host ($T.ok -f $ParamFile, (Split-Path $bak -Leaf))
if ($doPaper) {
    $cg = [PaperParams]::Find($ce, 'EquipParamGoods.param')
    $cr = [PaperParams]::Find($ce, 'ResourceItemParam.param')
    $cgr = [PaperParams]::RowOffsets($check, $cg.Start, $cg.Size)
    $cres = [PaperParams]::FindRowByGoods($check, $cr.Start, $cr.Size, 1000)
    Write-Host ($T.verify -f `
        [PaperParams]::U16($check, $cgr[1000] + 0x36), `
        [PaperParams]::U16($check, $cgr[1001] + 0x36), `
        [PaperParams]::U16($check, $cgr[3800] + 0x8C), `
        [PaperParams]::U32($check, $cres + 0x10), `
        [PaperParams]::F32($check, $cres + 0x20))
}
if ($doHp) {
    $ccg = [PaperParams]::Find($ce, 'CalcCorrectGraph.param')
    $menu = [PaperParams]::Find($ce, 'MenuParam.param')
    $ccgRows = [PaperParams]::RowOffsets($check, $ccg.Start, $ccg.Size)
    $menuRows = [PaperParams]::RowOffsets($check, $menu.Start, $menu.Size)
    Write-Host ($T.verifyHp -f `
        [PaperParams]::F32($check, $ccgRows[500] + 0x14), `
        [PaperParams]::F32($check, $ccgRows[500] + 0x18), `
        [PaperParams]::U32($check, $menuRows[0] + 0x08), `
        [PaperParams]::U32($check, $menuRows[0] + 0x9C))
}
