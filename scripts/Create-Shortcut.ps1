[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$ExecutablePath, [string]$DestinationDirectory)
$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
if (!(Test-Path -LiteralPath $executable -PathType Leaf) -or [IO.Path]::GetExtension($executable) -ne '.exe') { throw 'Choose an existing Project Shelf executable.' }
if (!$DestinationDirectory) { $DestinationDirectory = Split-Path -Parent $executable }
$directory = (Resolve-Path -LiteralPath $DestinationDirectory).Path
if (!(Test-Path -LiteralPath $directory -PathType Container)) { throw 'Choose an existing destination directory.' }
$shortcut = Join-Path $directory 'Project Shelf.lnk'
if (Test-Path -LiteralPath $shortcut) { throw 'Shortcut already exists; no file was overwritten.' }
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = $executable
$link.WorkingDirectory = Split-Path -Parent $executable
$link.IconLocation = "$executable,0"
$link.Description = 'Project Shelf'
$link.Save()
Write-Output $shortcut
