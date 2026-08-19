$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$bundlePath = Join-Path $PSScriptRoot "beefapi-codex-image2.sh"
$bundle = [System.IO.File]::ReadAllText($bundlePath)
$match = [regex]::Match(
    $bundle,
    "(?s)<<'BEEFAPI_IMAGE2_BUNDLE'\r?\n(?<payload>.*?)\r?\nBEEFAPI_IMAGE2_BUNDLE"
)
if (-not $match.Success) {
    throw "BeefAPI Image2 installer payload is missing."
}

$temporaryRoot = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    ("beefapi-image2." + [guid]::NewGuid().ToString("N"))
$installer = Join-Path $temporaryRoot "install.mjs"

try {
    [System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    $payload = $match.Groups["payload"].Value -replace "\s", ""
    [System.IO.File]::WriteAllBytes(
        $installer,
        [System.Convert]::FromBase64String($payload)
    )
    & node $installer @args
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
