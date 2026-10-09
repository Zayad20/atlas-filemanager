$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'run-release-perf.ps1'), [ref]$tokens, [ref]$errors
)
if ($errors.Count) { throw ($errors | Out-String) }
$definition = $ast.Find({ param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq 'Stop-WorkspaceSearchConsole'
}, $true)
. ([scriptblock]::Create($definition.Extent.Text))

$selected = Join-Path $root 'target\release-perf\atlas-search-service.exe'
$old = Join-Path $root 'target\release\atlas-search-service.exe'
$script:processes = @(
    [pscustomobject]@{ ProcessId = 1; ExecutablePath = $selected; CommandLine = '"service.exe" --console' },
    [pscustomobject]@{ ProcessId = 2; ExecutablePath = $old; CommandLine = '"service.exe" --console' },
    [pscustomobject]@{ ProcessId = 3; ExecutablePath = $old; CommandLine = '"service.exe"' },
    [pscustomobject]@{ ProcessId = 4; ExecutablePath = "$root\target-other\atlas-search-service.exe"; CommandLine = '"service.exe" --console' },
    [pscustomobject]@{ ProcessId = 5; ExecutablePath = 'C:\OtherCheckout\target\release\atlas-search-service.exe'; CommandLine = '"service.exe" --console' },
    [pscustomobject]@{ ProcessId = 6; ExecutablePath = $null; CommandLine = '"service.exe" --console' }
)
$script:stopped = [System.Collections.Generic.List[int]]::new()
# Mock process enumeration and termination: this test never stops real services.
function Get-CimInstance { param($ClassName, $Filter) $script:processes }
function Stop-Process {
    [CmdletBinding()]
    param([int]$Id, [switch]$Force)
    $script:stopped.Add($Id)
}

Stop-WorkspaceSearchConsole -OnlyExecutable $selected
if (($script:stopped -join ',') -ne '1') {
    throw "Build preparation stopped unrelated processes: $script:stopped"
}
$script:stopped.Clear()
Stop-WorkspaceSearchConsole
if (($script:stopped -join ',') -ne '1,2') {
    throw "Profile replacement stopped unrelated processes: $script:stopped"
}
Write-Host 'PASS: build unlocks only its executable; launch replaces only console processes from this checkout.'

$atlasDefinition = $ast.Find({ param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq 'Close-WorkspaceAtlas'
}, $true)
. ([scriptblock]::Create($atlasDefinition.Extent.Text))
$profile = 'release-perf'
$script:closed = [System.Collections.Generic.List[int]]::new()
function New-MockAtlas {
    param([int]$Id, [string]$Path, [bool]$CanClose = $true, [IntPtr]$MainWindowHandle = 123)
    $process = [pscustomobject]@{ Id = $Id; Path = $Path; CanClose = $CanClose; MainWindowHandle = $MainWindowHandle }
    $process | Add-Member -MemberType ScriptMethod -Name Refresh -Value {}
    $process | Add-Member -MemberType ScriptMethod -Name CloseMainWindow -Value {
        $script:closed.Add($this.Id)
        $this.CanClose
    }
    $process | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value {
        param($Timeout)
        $true
    }
    $process
}
$script:atlasProcesses = @(
    (New-MockAtlas 10 (Join-Path $root 'target\release-perf\atlas.exe')),
    (New-MockAtlas 11 (Join-Path $root 'target\release\atlas.exe')),
    (New-MockAtlas 12 'C:\OtherCheckout\target\release-perf\atlas.exe')
)
function Get-Process { param($Name) $script:atlasProcesses }
Close-WorkspaceAtlas
if (($script:closed -join ',') -ne '10') { throw 'Closed an unrelated Atlas instance.' }
$script:atlasProcesses = @((New-MockAtlas 13 (Join-Path $root 'target\release-perf\atlas.exe') $false))
$refused = $false
try { Close-WorkspaceAtlas } catch { $refused = $true }
if (-not $refused) { throw 'A refused normal close must abort the build.' }
Write-Host 'PASS: only the selected Atlas closes normally; a refused close aborts without force.'
$script:stopped.Clear()
$script:atlasProcesses = @(
    (New-MockAtlas 14 (Join-Path $root 'target\release-perf\atlas.exe') $false 0),
    (New-MockAtlas 15 'C:\OtherCheckout\target\release-perf\atlas.exe' $false 0)
)
Close-WorkspaceAtlas
if (($script:stopped -join ',') -ne '14') { throw 'Only this checkout background instance may be stopped.' }
Write-Host 'PASS: a background Atlas without a window no longer blocks rebuilding.'
