# Compile with the runner account; native acceptance must use ordinary user rights.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot

# Build under the runner's SDK/NuGet profile, before entering the test account.
function Build-NativeFixture {
    $buildDir = Join-Path $root ('target/native-fixture-' + [guid]::NewGuid().ToString('N'))
    $output = Join-Path $buildDir 'out'
    $obj = (Join-Path $buildDir 'obj') + [IO.Path]::DirectorySeparatorChar
    $project = Join-Path $root 'vendor/aevra/helper/tests/fixtures/background-controls/background-controls.csproj'
    & dotnet build $project -c Release --nologo -v minimal -o $output "-p:BaseIntermediateOutputPath=$obj" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Native .NET fixture build failed with exit code $LASTEXITCODE" }
    if (-not (Test-Path -LiteralPath (Join-Path $output 'background-controls.exe') -PathType Leaf)) {
        throw 'Native .NET fixture build did not produce background-controls.exe'
    }
    return $output
}

function Invoke-StandardAccountTest([string] $executable, [string] $fixtureDir) {
    if ($env:GITHUB_ACTIONS -ne 'true') {
        throw 'Elevated acceptance requires an ephemeral GitHub runner; run locally without elevation.'
    }
    $name = 'aevra-ci-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    $password = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(36)) + 'aA1!'
    $account = $null
    $desktopGrant = $null
    $granted = [Collections.Generic.List[string]]::new()
    $runDir = Join-Path $root ('target/native-standard-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $runDir | Out-Null
    try {
        $account = New-LocalUser -Name $name -Password (ConvertTo-SecureString $password -AsPlainText -Force) -Description 'Ephemeral Aevra CI native acceptance'
        if (-not (Get-LocalGroupMember -SID 'S-1-5-32-545' | Where-Object { $_.SID -eq $account.SID })) {
            Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $name
        }
        foreach ($entry in @(@($root, 'ReadAndExecute'), @((Join-Path $root 'target'), 'Modify'))) {
            $path = $entry[0]
            $acl = Get-Acl -LiteralPath $path
            $rule = [Security.AccessControl.FileSystemAccessRule]::new($account.SID, $entry[1], 'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $acl.AddAccessRule($rule)
            $granted.Add($path)
            Set-Acl -LiteralPath $path -AclObject $acl
        }
        $desktopGrant = [AevraMediumToken]::GrantDesktop($account.SID.Value)
        $settings = @{ executable = $executable; log = (Join-Path $runDir 'native.log'); dotnetRoot = $env:DOTNET_ROOT; fixtureDir = $fixtureDir }
        $settings | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runDir 'settings.json')
        $bootstrap = Join-Path $runDir 'run.ps1'
        @'
$ErrorActionPreference = 'Stop'
$config = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'settings.json') | ConvertFrom-Json
$env:AEVRA_SKIP_NATIVE_FIXTURE = ''
$env:AEVRA_REQUIRE_NATIVE_FIXTURE = '1'
$env:AEVRA_NATIVE_FIXTURE_DIR = $config.fixtureDir
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
if ($config.dotnetRoot) { $env:DOTNET_ROOT = $config.dotnetRoot; $env:PATH = "$($config.dotnetRoot);$env:PATH" }
$code = 1
try { & $config.executable --nocapture *> $config.log; $code = $LASTEXITCODE }
catch { $_ | Out-String | Add-Content -LiteralPath $config.log }
exit $code
'@ | Set-Content -LiteralPath $bootstrap
        $shell = (Get-Process -Id $PID).Path
        try { $code = [AevraMediumToken]::RunAsStandardUser($shell, $bootstrap, $root, $name, $password) }
        finally { if (Test-Path -LiteralPath $settings.log) { Get-Content -LiteralPath $settings.log | Out-Host } }
        return $code
    } finally {
        # Try every cleanup even when a test, launch or ACL operation fails.
        $failures = [Collections.Generic.List[string]]::new()
        if ($desktopGrant) { try { $desktopGrant.Dispose() } catch { $failures.Add($_.Exception.Message) } }
        if ($account) {
            for ($i = $granted.Count - 1; $i -ge 0; $i--) {
                try {
                    $acl = Get-Acl -LiteralPath $granted[$i]
                    $acl.PurgeAccessRules($account.SID)
                    Set-Acl -LiteralPath $granted[$i] -AclObject $acl
                } catch { $failures.Add($_.Exception.Message) }
            }
            try { Remove-LocalUser -SID $account.SID } catch { $failures.Add($_.Exception.Message) }
        }
        $password = $null
        if ($failures.Count) { throw ('Native acceptance cleanup failed: ' + ($failures -join '; ')) }
    }
}

Push-Location $root
try {
    $fixtureDir = Build-NativeFixture
    $messages = & cargo test -p desktop-mcp --test native_windows --locked --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'Native fixture compilation failed' }
    $artifacts = @($messages | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object {
        $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'native_windows' -and $_.executable
    })
    if ($artifacts.Count -ne 1) { throw 'Expected exactly one native Windows test executable' }
    Add-Type -Path @((Join-Path $PSScriptRoot 'windows-medium-token.cs'), (Join-Path $PSScriptRoot 'windows-standard-logon.cs'))
    [AevraMediumToken]::ValidateDesktopAccess()
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        $code = Invoke-StandardAccountTest $artifacts[0].executable $fixtureDir
    } else {
        $env:AEVRA_SKIP_NATIVE_FIXTURE = ''
        $env:AEVRA_REQUIRE_NATIVE_FIXTURE = '1'
        $env:AEVRA_NATIVE_FIXTURE_DIR = $fixtureDir
        $code = [AevraMediumToken]::Run($artifacts[0].executable, $root)
    }
    if ($code -ne 0) { throw "Native Windows acceptance failed with exit code $code" }
} finally { Pop-Location }
