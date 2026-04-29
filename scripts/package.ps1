param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Arguments
)

$ErrorActionPreference = 'Stop'
$rootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$scriptPath = Join-Path $rootDir 'release.py'

if (Get-Command python3 -ErrorAction SilentlyContinue) {
  & python3 $scriptPath @Arguments
  exit $LASTEXITCODE
}

if (Get-Command python -ErrorAction SilentlyContinue) {
  & python $scriptPath @Arguments
  exit $LASTEXITCODE
}

if (Get-Command py -ErrorAction SilentlyContinue) {
  & py -3 $scriptPath @Arguments
  exit $LASTEXITCODE
}

throw 'python3 is required to run scripts/release.py'
