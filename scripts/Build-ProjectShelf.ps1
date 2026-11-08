[CmdletBinding()]
param([switch]$SkipInstall, [switch]$RunTests)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$previousLocation = Get-Location
function Require-Tool([string]$Name) {
  $tool = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if (!$tool) { throw "Required tool not found in PATH: $Name" }
  return $tool.Source
}
try {
  Set-Location -LiteralPath $projectRoot
  $node = Require-Tool 'node.exe'
  $npm = Require-Tool 'npm.cmd'
  $cargo = Require-Tool 'cargo.exe'
  $major = & $node -p 'Number.parseInt(process.versions.node)'
  if ($LASTEXITCODE -ne 0 -or [int]$major -lt 24) { throw 'Node.js 24 or newer is required.' }
  if (!$SkipInstall) {
    & $npm ci --ignore-scripts --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'Dependency installation failed.' }
  }
  if ($RunTests) {
    & $npm test
    if ($LASTEXITCODE -ne 0) { throw 'Node tests failed.' }
    & $cargo test --manifest-path src-tauri\Cargo.toml --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests did not complete successfully.' }
  }
  & $npm run desktop:build
  if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed.' }
  Write-Output 'Build complete. See src-tauri/target/release and bundle/nsis.'
} finally { Set-Location -LiteralPath $previousLocation }
