<#
  make_bat.ps1 —— 生成自解压一键脚本：
    dist/zh/纸人挂.bat     ← 中文界面（GBK 编码）
    dist/en/paperman.bat   ← English UI (GBK-encoded, ASCII-only header/echoes)

  做法：把 tools/patch_paper_params.ps1 原样嵌进 bat 尾部，bat 运行时用 PowerShell
  把自己第 N 行之后的内容抽到 %TEMP%\*.ps1 交给 PowerShell 执行。
  这样发布包只需 dinput8.dll + battle_instinct.cfg + 一个 .bat（无需任何 .ps1）。

  界面语言通过环境变量 PAPERDOLL_LANG 传给内嵌脚本（en / zh，缺省按系统区域）。

  改完 tools/patch_paper_params.ps1 后跑一次本脚本（或 just bat）即可重新生成两个 bat。
#>
[CmdletBinding()]
param(
    [string]$Source = '',
    [string]$RepoRoot = ''
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Source) { $Source = Join-Path $here 'patch_paper_params.ps1' }
if (-not $RepoRoot) { $RepoRoot = Split-Path -Parent $here }
if (-not (Test-Path $Source)) { throw "找不到源脚本: $Source" }
$psCode = ([System.IO.File]::ReadAllText($Source)) -replace "`r`n", "`n"

# 目标：目录 / 文件名 / 界面语言 / 头部文案
$targets = @(
    [pscustomobject]@{
        Dir   = 'zh'
        File  = '纸人挂.bat'
        Lang  = 'zh'
        Title = '只狼 · 纸人挂'
    },
    [pscustomobject]@{
        Dir   = 'en'
        File  = 'paperman.bat'
        Lang  = 'en'
        Title = 'Sekiro - Paper Doll Tool'
    }
)

function New-Header([string]$title, [string]$lang) {
    $en = ($lang -eq 'en')
    $h = @(
        '@echo off',
        'setlocal',
        'chcp 936 >nul',
        ('title ' + $title),
        'set "HERE=%~dp0"',
        'set "PAPERDOLL_CFG=%HERE%battle_instinct.cfg"',
        ('set "PAPERDOLL_LANG=' + $lang + '"'),
        'if /i "%~1"=="revert" set "PAPERDOLL_REVERT=1"',
        'if /i "%~1"=="rollback" set "PAPERDOLL_REVERT=1"',
        'echo ==========================================================',
        ('echo   ' + $title),
        'echo ----------------------------------------------------------'
    )
    if ($en) {
        $h += @(
            'echo   Reads these lines from battle_instinct.cfg (same folder):',
            'echo     paper cap fix: on / off',
            'echo     paper initial cap: 25    per skill bonus: 5    drift: 9',
            'echo   "paper cap fix: off" = write nothing.',
            'echo   Edit the cfg, then double-click this file. Roll back: paperman.bat revert',
            'echo ==========================================================',
            'echo.',
            'if not exist "%PAPERDOLL_CFG%" (',
            '  echo [ERROR] battle_instinct.cfg not found next to this file.',
            '  goto done',
            ')',
            'set "TMPPS=%TEMP%\zhiren_%RANDOM%%RANDOM%.ps1"',
            'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -LiteralPath ''%~f0'' -Encoding Default | Select-Object -Skip __SKIP__ | Set-Content -LiteralPath ''%TMPPS%'' -Encoding Default"',
            'if not exist "%TMPPS%" (',
            '  echo [ERROR] could not unpack the temporary script.',
            '  goto done',
            ')',
            'powershell -NoProfile -ExecutionPolicy Bypass -File "%TMPPS%" -Apply',
            'set "RC=%ERRORLEVEL%"',
            'del "%TMPPS%" >nul 2>nul',
            'if not "%RC%"=="0" echo [WARN] script exit code %RC%',
            'goto done',
            ':done',
            'echo.',
            'echo Done. Press any key to exit.',
            'pause >nul',
            'endlocal',
            'exit /b',
            '',
            'rem ==== embedded PowerShell below (self-extracting; regenerate with tools/make_bat.ps1) ===='
        )
    } else {
        $h += @(
            'echo   读取同目录 battle_instinct.cfg 里的这几行：',
            'echo     纸人上限功能: 开 / 关',
            'echo     纸人初始上限: 25     技能增加上限: 5     纸人漂流: 9',
            'echo   「纸人上限功能: 关」= 什么都不写。',
            'echo   改完 cfg 双击本文件；回滚：纸人挂.bat revert',
            'echo ==========================================================',
            'echo.',
            'if not exist "%PAPERDOLL_CFG%" (',
            '  echo [错误] 同目录找不到 battle_instinct.cfg，请把它和本文件放在一起。',
            '  goto done',
            ')',
            'set "TMPPS=%TEMP%\zhiren_%RANDOM%%RANDOM%.ps1"',
            'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -LiteralPath ''%~f0'' -Encoding Default | Select-Object -Skip __SKIP__ | Set-Content -LiteralPath ''%TMPPS%'' -Encoding Default"',
            'if not exist "%TMPPS%" (',
            '  echo [错误] 解出临时脚本失败。',
            '  goto done',
            ')',
            'powershell -NoProfile -ExecutionPolicy Bypass -File "%TMPPS%" -Apply',
            'set "RC=%ERRORLEVEL%"',
            'del "%TMPPS%" >nul 2>nul',
            'if not "%RC%"=="0" echo [提示] 脚本退出码 %RC%',
            'goto done',
            ':done',
            'echo.',
            'echo 完成。按任意键退出。',
            'pause >nul',
            'endlocal',
            'exit /b',
            '',
            'rem ============ 以下为内嵌 PowerShell 代码（自解执行；改 tools/patch_paper_params.ps1 后跑 tools/make_bat.ps1 重新生成）============'
        )
    }
    return ($h -join "`n")
}

$gbk = [System.Text.Encoding]::GetEncoding(936)
foreach ($t in $targets) {
    $header = New-Header -title $t.Title -lang $t.Lang
    $skip = $header.Split("`n").Count            # Select-Object -Skip 需要「跳过多少行」
    $header = $header.Replace('__SKIP__', [string]$skip)
    $bat = ($header + "`n" + $psCode)
    $bat = ($bat -replace "`r`n", "`n") -replace "`n", "`r`n"

    $outDir = Join-Path $RepoRoot ("dist\{0}" -f $t.Dir)
    if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
    $file = Join-Path $outDir $t.File
    [System.IO.File]::WriteAllText($file, $bat, $gbk)
    Write-Host ("生成 {0} ({1} bytes, lang={2}, 内嵌 PS 起于第 {3} 行)" -f $file, (Get-Item -LiteralPath $file).Length, $t.Lang, $skip)
}