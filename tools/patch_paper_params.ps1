<#
  patch_paper_params.ps1 - 纸人（Spirit Emblem / 形代）相关 param 一键修改
  ------------------------------------------------------------------------
  不改 DLL、不覆盖别人的 mod：直接对 Mod Engine 加载的 regulation 参数包
  mods\param\gameparam\gameparam.parambnd.dcx 做字段级补丁（自动备份、可回滚）。

  字段来源（本机两套 mod 包对比 + 逐行 dump 实测）：
    EquipParamGoods  id=1000  纸人      row+0x36 (u16) = 纸人上限         vanilla 15
    EquipParamGoods  id=1001  临时纸人  row+0x36 (u16) = 临时纸人上限     vanilla 5
    EquipParamGoods  id=3800  纸人漂流  row+0x8C (u16) = 一次给多少临时   vanilla 5
    ResourceItemParam goodsId=1000 行  +0x10 (u32)    = 开局携带纸人数    vanilla 6
    ResourceItemParam goodsId=1000 行  +0x14/+0x18/+0x1C/+0x20 (f32)
        = 基础上限/4、满上限/4、技能个数、满上限（推断，用于「每个技能 +N」）

  用法：
      只看不改:  powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1
      应用:      powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Apply
      回滚:      powershell -ExecutionPolicy Bypass -File tools\patch_paper_params.ps1 -Revert
      自定义:    ... -Apply -InitialPaper 30 -BaseCap 30 -PerSkillBonus 10 -TempCap 15 -DriftAmount 15
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
    [switch]$SkipGrowthFloats,
    [switch]$Apply,
    [switch]$Force,
    [switch]$Revert
)

$ErrorActionPreference = 'Stop'

# ---- 从 battle_instinct.cfg 读三个可调项（cfg 优先，缺省用上面的默认值）----
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

function Find-GameDir {
    $cands = @(
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
    if (-not $gameDir) { throw "找不到 Sekiro 目录，请用 -ParamFile 指定 gameparam.parambnd.dcx" }
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
if (-not $Revert -and -not $Force -and $capFixOn -eq 0) {
    Write-Host '当前 cfg 里「纸人上限功能: 关」→ 跳过 param 写入（想强制加 -Force，想回滚加 -Revert）'
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
if (-not (Test-Path $ParamFile)) { throw "找不到参数文件: $ParamFile" }

if ($Revert) {
    $baks = Get-ChildItem ($ParamFile + '.bak-*') -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending
    if (-not $baks) { throw "没有找到备份 ($ParamFile.bak-*)，无法回滚" }
    $src = $baks[0].FullName
    Copy-Item $src $ParamFile -Force
    Write-Host "已回滚: $src -> $ParamFile"
    return
}

$orig = [System.IO.File]::ReadAllBytes($ParamFile)
$bnd  = [PaperParams]::Expand($orig)
$entries = [PaperParams]::Entries($bnd)

$goods = [PaperParams]::Find($entries, 'EquipParamGoods.param')
$res   = [PaperParams]::Find($entries, 'ResourceItemParam.param')
if (-not $goods) { throw '参数包里没有 EquipParamGoods.param' }
if (-not $res)   { throw '参数包里没有 ResourceItemParam.param' }

$gRows = [PaperParams]::RowOffsets($bnd, $goods.Start, $goods.Size)
$maxCap = $BaseCap + $PerSkillBonus * $SkillCount

function Set-U16([int]$row, [int]$off, [int]$val, [string]$label) {
    $old = [PaperParams]::U16($bnd, $row + $off)
    [PaperParams]::W16($bnd, $row + $off, $val)
    Write-Host ("  {0,-34} {1} -> {2}" -f $label, $old, $val)
}
function Set-U32([int]$row, [int]$off, [int]$val, [string]$label) {
    $old = [PaperParams]::U32($bnd, $row + $off)
    [PaperParams]::W32($bnd, $row + $off, [uint32]$val)
    Write-Host ("  {0,-34} {1} -> {2}" -f $label, $old, $val)
}
function Set-F32([int]$row, [int]$off, [single]$val, [string]$label) {
    $old = [PaperParams]::F32($bnd, $row + $off)
    [PaperParams]::WF32($bnd, $row + $off, $val)
    Write-Host ("  {0,-34} {1} -> {2}" -f $label, $old, $val)
}

Write-Host "目标文件: $ParamFile"
Write-Host "计划修改:"
if ($gRows.ContainsKey(1000)) {
    Set-U16 $gRows[1000] 0x36 $BaseCap 'EquipParamGoods 1000 纸人上限'
} else { Write-Host '  !! EquipParamGoods 里没有 id=1000' }
if ($gRows.ContainsKey(1001)) {
    Set-U16 $gRows[1001] 0x36 $TempCap 'EquipParamGoods 1001 临时纸人上限'
} else { Write-Host '  !! EquipParamGoods 里没有 id=1001' }
if ($gRows.ContainsKey(3800)) {
    Set-U16 $gRows[3800] 0x8C $DriftAmount 'EquipParamGoods 3800 纸人漂流给的数量'
} else { Write-Host '  !! EquipParamGoods 里没有 id=3800（纸人漂流）' }

$resRow = [PaperParams]::FindRowByGoods($bnd, $res.Start, $res.Size, 1000)
if ($resRow -ge 0) {
    Set-U32 $resRow 0x10 $InitialPaper 'ResourceItemParam 1000 初始纸人'
    if (-not $SkipGrowthFloats) {
        Write-Host '  (以下 4 个浮点是「每个技能 +N」的唯一 param 侧入口，属推断字段，-SkipGrowthFloats 可关)'
        Set-F32 $resRow 0x14 ([single]($BaseCap / 4.0))   'ResourceItemParam 1000 上限成长 f1'
        Set-F32 $resRow 0x18 ([single]($maxCap / 4.0))    'ResourceItemParam 1000 上限成长 f2'
        Set-F32 $resRow 0x1C ([single]$SkillCount)            'ResourceItemParam 1000 上限技能个数'
        Set-F32 $resRow 0x20 ([single]$maxCap)            'ResourceItemParam 1000 满级上限'
    }
} else { Write-Host '  !! ResourceItemParam 里没有 goodsId=1000 的行' }

if (-not $Apply) {
    Write-Host ''
    Write-Host '（演练模式：没有写入任何文件；加 -Apply 才真正写入）'
    return
}

$bak = $ParamFile + '.bak-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
Copy-Item $ParamFile $bak -Force
$packed = [PaperParams]::Pack($orig, $bnd)
[System.IO.File]::WriteAllBytes($ParamFile, $packed)

# verify round trip
$check = [PaperParams]::Expand([System.IO.File]::ReadAllBytes($ParamFile))
$ce = [PaperParams]::Entries($check)
$cg = [PaperParams]::Find($ce, 'EquipParamGoods.param')
$cr = [PaperParams]::Find($ce, 'ResourceItemParam.param')
$cgr = [PaperParams]::RowOffsets($check, $cg.Start, $cg.Size)
$cres = [PaperParams]::FindRowByGoods($check, $cr.Start, $cr.Size, 1000)
Write-Host ''
Write-Host ("已写入: {0}  (备份 {1})" -f $ParamFile, (Split-Path $bak -Leaf))
Write-Host ("校验: 纸人上限={0} 临时上限={1} 漂流给={2} 初始纸人={3} 满上限={4}" -f `
    [PaperParams]::U16($check, $cgr[1000] + 0x36), `
    [PaperParams]::U16($check, $cgr[1001] + 0x36), `
    [PaperParams]::U16($check, $cgr[3800] + 0x8C), `
    [PaperParams]::U32($check, $cres + 0x10), `
    [PaperParams]::F32($check, $cres + 0x20))
