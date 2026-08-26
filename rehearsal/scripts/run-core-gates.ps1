param(
    [int]$Port = 55434,
    [string]$PostgresBin = "C:\Program Files\PostgreSQL\17\bin"
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$data = Join-Path $root "rehearsal\.postgres-data"
$log = Join-Path $root "rehearsal\.postgres.log"
$database = "lynshen_rehearsal"
$postgresUrl = "postgres://postgres@127.0.0.1:$Port/$database"
$startedHere = $false

function Invoke-Checked {
    param([string]$Executable, [string[]]$Arguments)
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable exited with code $LASTEXITCODE"
    }
}

try {
    foreach ($binary in @("initdb.exe", "pg_ctl.exe", "pg_isready.exe", "createdb.exe", "psql.exe")) {
        if (-not (Test-Path -LiteralPath (Join-Path $PostgresBin $binary))) {
            throw "PostgreSQL binary is missing: $(Join-Path $PostgresBin $binary)"
        }
    }

    if (-not (Test-Path -LiteralPath (Join-Path $data "PG_VERSION"))) {
        Invoke-Checked (Join-Path $PostgresBin "initdb.exe") @(
            "-D", $data, "-A", "trust", "-U", "postgres", "--no-locale", "--encoding=UTF8"
        )
    }

    & (Join-Path $PostgresBin "pg_isready.exe") -h 127.0.0.1 -p $Port -d $database | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Invoke-Checked (Join-Path $PostgresBin "pg_ctl.exe") @(
            "-D", $data, "-l", $log, "-o", "-h 127.0.0.1 -p $Port", "start"
        )
        $startedHere = $true
    }

    $databaseExists = & (Join-Path $PostgresBin "psql.exe") -h 127.0.0.1 -p $Port -U postgres -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname = '$database'"
    if ($LASTEXITCODE -ne 0) {
        throw "database existence query exited with code $LASTEXITCODE"
    }
    if ($databaseExists -ne "1") {
        Invoke-Checked (Join-Path $PostgresBin "createdb.exe") @(
            "-h", "127.0.0.1", "-p", $Port.ToString(), "-U", "postgres", $database
        )
    }

    Push-Location $root
    try {
        $env:LYNSHEN_REHEARSAL_POSTGRES_URL = $postgresUrl
        Invoke-Checked "cargo" @(
            "test", "--manifest-path", "rehearsal/Cargo.toml", "--all-targets", "--", "--test-threads=1"
        )
        Invoke-Checked "cargo" @(
            "clippy", "--manifest-path", "rehearsal/Cargo.toml", "--all-targets", "--", "-D", "warnings"
        )
        Invoke-Checked "cargo" @(
            "fmt", "--manifest-path", "rehearsal/Cargo.toml", "--check"
        )
        Invoke-Checked "git" @("diff", "--check")

        $env:LYNSHEN_REHEARSAL_POSTGRES_VERIFIED = "1"
        $env:LYNSHEN_REHEARSAL_GIT_COMMIT = (& git rev-parse HEAD).Trim()
        Invoke-Checked "cargo" @(
            "run", "--manifest-path", "rehearsal/Cargo.toml", "--bin", "lynshen-rehearsal", "--",
            "gate-summary", "--output", "rehearsal/evidence/gate-summary.json"
        )
    }
    finally {
        Pop-Location
    }
}
finally {
    if ($startedHere) {
        & (Join-Path $PostgresBin "pg_ctl.exe") -D $data stop -m fast | Out-Null
    }
}
