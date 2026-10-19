# Exercise the orchestration without creating users or changing real ACLs.
$ErrorActionPreference = 'Stop'
$tokens = $null
$errors = $null
$source = Join-Path $PSScriptRoot 'test-native-windows.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors -join '; ') }
$function = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-StandardAccountTest' }, $true)
if (-not $function) { throw 'Standard account orchestration missing' }
. ([scriptblock]::Create($function.Extent.Text))
$buildFunction = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Build-NativeFixture' }, $true)
if (-not $buildFunction) { throw 'Fixture must be built before switching to the standard account' }
. ([scriptblock]::Create($buildFunction.Extent.Text))

Add-Type @'
using System;
public static class AevraMediumToken {
    public static bool FailLaunch;
    public static int Disposals;
    sealed class Grant : IDisposable { public void Dispose() { Disposals++; } }
    public static IDisposable GrantDesktop(string sid) { return new Grant(); }
    public static int RunAsStandardUser(string exe, string script, string dir, string user, string password) {
        if (FailLaunch) throw new InvalidOperationException("simulated logon failure");
        if (password.Length < 40) throw new Exception("weak generated password");
        return 17;
    }
}
'@
$events = [Collections.Generic.List[string]]::new()
$script:aclWrites = 0
$script:failAclAt = 0
function New-LocalUser { param($Name, $Password, $Description) $events.Add('create'); return @{ SID = [Security.Principal.SecurityIdentifier]::new('S-1-5-21-111-222-333-1001') } }
function Get-LocalGroupMember { param($SID) return @() }
function Add-LocalGroupMember { param($SID, $Member) if ($SID -ne 'S-1-5-32-545') { throw 'Must use Users group' }; $events.Add('group') }
function Get-Acl { param($LiteralPath) return [Security.AccessControl.DirectorySecurity]::new() }
function Set-Acl {
    param($LiteralPath, $AclObject)
    $events.Add('acl')
    $script:aclWrites++
    if ($script:failAclAt -eq $script:aclWrites) { throw 'simulated ACL failure' }
}
function Remove-LocalUser { param($SID) $events.Add('remove') }
function Assert($condition, $message) { if (-not $condition) { throw $message } }

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('aevra-launcher-test-' + [guid]::NewGuid().ToString('N'))
$root = $scratch
$oldActions = $env:GITHUB_ACTIONS
[IO.Directory]::CreateDirectory($root) | Out-Null
try {
    $env:GITHUB_ACTIONS = 'true'
    $script:dotnetExit = 0
    $script:omitFixture = $false
    function dotnet {
        $outputIndex = [Array]::IndexOf($args, '-o')
        if ($outputIndex -lt 0) { throw 'Fixture build must use an explicit output directory' }
        $output = $args[$outputIndex + 1]
        [IO.Directory]::CreateDirectory($output) | Out-Null
        if (-not $script:omitFixture) { Set-Content -LiteralPath (Join-Path $output 'background-controls.exe') -Value 'test stub' }
        $global:LASTEXITCODE = $script:dotnetExit
    }
    $fixtureDir = Build-NativeFixture
    Assert (Test-Path -LiteralPath (Join-Path $fixtureDir 'background-controls.exe')) 'Prepared fixture missing'
    $otherFixture = Build-NativeFixture
    Assert ($fixtureDir -ne $otherFixture) 'Every invocation must build into a fresh directory'
    $script:dotnetExit = 23
    $failed = $false
    try { Build-NativeFixture } catch { $failed = $_.Exception.Message -like '*23*' }
    Assert $failed 'Fixture compiler failure must propagate before launching a test'
    $script:dotnetExit = 0
    $script:omitFixture = $true
    $failed = $false
    try { Build-NativeFixture } catch { $failed = $_.Exception.Message -like '*background-controls.exe*' }
    Assert $failed 'Compiler success without fixture executable must fail'
    $script:omitFixture = $false
    $code = Invoke-StandardAccountTest 'C:/fixture/native.exe' $fixtureDir
    Assert ($code -eq 17) 'Must propagate native test failure'
    Assert ($events.Contains('remove') -and [AevraMediumToken]::Disposals -eq 1) 'Success cleanup missing'
    Assert (@($events | Where-Object { $_ -eq 'acl' }).Count -eq 4) 'Both directory grants must be removed'
    $bootstrap = Get-ChildItem -LiteralPath (Join-Path $root 'target') -Filter run.ps1 -Recurse | Select-Object -First 1
    [Management.Automation.Language.Parser]::ParseFile($bootstrap.FullName, [ref]$tokens, [ref]$errors) | Out-Null
    Assert ($errors.Count -eq 0) 'Generated child script must parse'
    $settings = Get-Content -Raw -LiteralPath (Join-Path $bootstrap.DirectoryName 'settings.json') | ConvertFrom-Json
    Assert ($settings.fixtureDir -eq $fixtureDir) 'Standard-user handoff must retain the prepared fixture path'
    Assert ((Get-Content -Raw -LiteralPath $bootstrap.FullName).Contains('$env:AEVRA_NATIVE_FIXTURE_DIR = $config.fixtureDir')) 'Child must consume the prepared fixture'
    # Execute the generated child script too; no compiler is needed here.
    $stub = Join-Path $root 'native-stub.ps1'
    @'
if ($env:AEVRA_REQUIRE_NATIVE_FIXTURE -ne '1') { exit 51 }
if (-not (Test-Path -LiteralPath (Join-Path $env:AEVRA_NATIVE_FIXTURE_DIR 'background-controls.exe'))) { exit 52 }
Write-Output 'Prepared fixture consumed without building'
exit 17
'@ | Set-Content -LiteralPath $stub
    $settings.executable = $stub
    $settings | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $bootstrap.DirectoryName 'settings.json')
    & (Get-Process -Id $PID).Path -NoProfile -File $bootstrap.FullName
    Assert ($LASTEXITCODE -eq 17) 'Generated child must consume the fixture and propagate test failure'
    Assert ((Get-Content -Raw -LiteralPath $settings.log).Contains('Prepared fixture consumed without building')) 'Child diagnostics must survive the handoff'

    $events.Clear()
    [AevraMediumToken]::FailLaunch = $true
    $failed = $false
    try { Invoke-StandardAccountTest 'C:/fixture/native.exe' $fixtureDir } catch { $failed = $_.Exception.Message -like '*simulated logon failure*' }
    Assert $failed 'Logon failure must propagate'
    Assert ($events.Contains('remove') -and [AevraMediumToken]::Disposals -eq 2) 'Launch failure cleanup missing'

    $events.Clear()
    $script:failAclAt = $script:aclWrites + 1
    $failed = $false
    try { Invoke-StandardAccountTest 'C:/fixture/native.exe' $fixtureDir } catch { $failed = $_.Exception.Message -like '*simulated ACL failure*' }
    Assert ($failed -and $events.Contains('remove')) 'ACL setup failure must still remove the account'
    Assert (@($events | Where-Object { $_ -eq 'acl' }).Count -eq 2) 'Partially applied ACL must be cleaned up'

    $events.Clear()
    $env:GITHUB_ACTIONS = 'false'
    $refused = $false
    try { Invoke-StandardAccountTest 'C:/fixture/native.exe' $fixtureDir } catch { $refused = $_.Exception.Message -like '*ephemeral GitHub runner*' }
    Assert ($refused -and $events.Count -eq 0) 'Must refuse account creation outside hosted CI'
    Write-Output 'Launcher orchestration: fixture preparation, child handoff, exit propagation, cleanup and local refusal passed.'
} finally {
    $env:GITHUB_ACTIONS = $oldActions
    $resolved = [IO.Path]::GetFullPath($scratch)
    if ([IO.Path]::GetDirectoryName($resolved) -ne [IO.Path]::GetTempPath().TrimEnd('\', '/')) { throw 'Unsafe scratch cleanup target' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
# An intentional child failure sets LASTEXITCODE to 17. All assertions passed,
# and cleanup succeeded, so explicitly report success to the PowerShell host.
exit 0
