<#
.SYNOPSIS
  Windows 上运行 Beefex 的 Rust 测试（绕开 comctl32 v6 清单缺失导致的 0xC0000139）。

.DESCRIPTION
  cargo test 构建的测试二进制没有 Common-Controls v6 应用清单，而依赖静态导入了
  comctl32!TaskDialogIndirect（仅 v6 导出）→ 测试 exe 加载即 STATUS_ENTRYPOINT_NOT_FOUND。
  本脚本：1) 先只构建测试二进制；2) 用 Windows SDK mt.exe 给 beefex-* test harness 合并
  Common-Controls v6 清单；3) 再运行测试。只修改测试产物，不触碰 Tauri 应用主程序的清单。
  详见 src-tauri/tests-common-controls.manifest。

.EXAMPLE
  ./scripts/win-cargo-test.ps1
  ./scripts/win-cargo-test.ps1 --lib
  ./scripts/win-cargo-test.ps1 build_error_arm_message
  ./scripts/win-cargo-test.ps1 --lib chat::agent::loop_tests
#>
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repo 'src-tauri\Cargo.toml'
$ccManifest = Join-Path $repo 'src-tauri\tests-common-controls.manifest'
$depsDir = Join-Path $repo 'src-tauri\target\debug\deps'

Write-Host '[win-cargo-test] 1/3 构建测试二进制 (--no-run)...' -ForegroundColor Cyan
cargo test --manifest-path $manifestPath @args --no-run
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host '[win-cargo-test] 2/3 合并 Common-Controls v6 测试清单...' -ForegroundColor Cyan
$mt = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\mt.exe' -ErrorAction SilentlyContinue |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $mt) {
  throw 'Windows SDK mt.exe is required to run Beefex Rust tests on Windows'
}

$testExecutables = Get-ChildItem "$depsDir\beefex-*.exe" -ErrorAction SilentlyContinue
if (-not $testExecutables) {
  throw 'cargo test --no-run did not produce a Beefex test executable'
}
foreach ($testExecutable in $testExecutables) {
  & $mt.FullName -nologo -manifest $ccManifest "-outputresource:$($testExecutable.FullName);#1"
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Write-Host '[win-cargo-test] 3/3 运行测试...' -ForegroundColor Cyan
cargo test --manifest-path $manifestPath @args
exit $LASTEXITCODE
