# SP-04 (#44) rerun on the Windows reference laptop (i5-1235U or Ryzen 5 7530U). See docs/spikes/SP-04.md.
#
#   powershell -ExecutionPolicy Bypass -File spikes\sp-04\run.ps1                 # from the repository root
#   powershell -ExecutionPolicy Bypass -File spikes\sp-04\run.ps1 -AudioDir C:\sp04-audio-from-mac
#
# Needs: Rust stable (1.88 or later) with the MSVC toolchain (Visual Studio Build Tools, "Desktop
# development with C++"), Windows 10 or 11 x64. About 1.1 GB of downloads go into -Work.
# -AudioDir: a folder with clip_{2,5,10,30}s.wav and ref_{2,5,10,30}.txt copied from a Mac run.sh
# run (best for comparison). Without it, the script makes the clips with System.Speech.
# Exit code: 0 = all four SP-04 criteria pass, 1 = one or more fail, 2 = error.
param(
    [string]$Work = "$env:USERPROFILE\sp04-work",
    [string]$AudioDir = "",
    [int]$Threads = 0,
    [int]$Runs = 20
)
$ErrorActionPreference = "Stop"
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Version = "2.0.1"
$LibsName = "sherpa-onnx-v$Version-win-x64-static-MT-Release-lib.tar.bz2"
$LibsUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$Version/$LibsName"
$LibsSha = "23e7b986b197eda0bbc47ff3d7d5c87afd6ef2b9077214ba26b0f1ec52ef0e66"
$ModelName = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"
$ModelUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/$ModelName.tar.bz2"
$ModelSha = "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9"

foreach ($d in "libs", "model", "audio", "out") { New-Item -ItemType Directory -Force -Path (Join-Path $Work $d) | Out-Null }

function Fetch($Url, $Dest, $Sha) {
    $ok = (Test-Path $Dest) -and ((Get-FileHash -Algorithm SHA256 $Dest).Hash.ToLower() -eq $Sha)
    if (-not $ok) {
        Write-Host "download $Url"
        curl.exe -fL --retry 3 -C - -o $Dest $Url
        if ($LASTEXITCODE -ne 0) { throw "download failed: $Url" }
    }
    $got = (Get-FileHash -Algorithm SHA256 $Dest).Hash.ToLower()
    if ($got -ne $Sha) { Write-Host "SHA-256 mismatch for ${Dest}: got $got, want $Sha"; exit 2 }
    Write-Host "sha256 OK $got $(Split-Path -Leaf $Dest)"
}

Write-Host "== 1. machine"
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$cpu | Format-List Name, NumberOfCores, NumberOfLogicalProcessors, MaxClockSpeed
$all = [int]$cpu.NumberOfCores
if ($Threads -le 0) {
    # Physical performance cores (CAP-48). The i5-1235U has 2 P-cores and 8 E-cores.
    if ($cpu.Name -match "i5-1235U") { $Threads = 2 }
    elseif ($cpu.Name -match "Ryzen 5 7530U") { $Threads = 6 }
    else { Write-Host "Unknown CPU: pass -Threads <P-core count>"; exit 2 }
}
Write-Host "P-core threads=$Threads physical cores=$all"
Get-CimInstance Win32_OperatingSystem | Format-List Caption, Version, TotalVisibleMemorySize
Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Format-List BatteryStatus
Write-Host "Plug in AC power and pick the 'Best performance' power mode before you continue."

Write-Host "== 2. downloads (pinned SHA-256)"
Fetch $LibsUrl (Join-Path $Work "libs\$LibsName") $LibsSha
$ModelTar = Join-Path $Work "model\$ModelName.tar.bz2"
Fetch $ModelUrl $ModelTar $ModelSha
$M = Join-Path $Work "model-x\$ModelName"
if (-not (Test-Path (Join-Path $M "encoder.int8.onnx"))) {
    New-Item -ItemType Directory -Force -Path (Join-Path $Work "model-x") | Out-Null
    tar.exe -xjf $ModelTar -C (Join-Path $Work "model-x")
    if ($LASTEXITCODE -ne 0) { throw "extract failed" }
}

Write-Host "== 3. test audio"
if ($AudioDir -ne "") {
    $A = $AudioDir
} else {
    $A = Join-Path $Work "audio"
    Add-Type -AssemblyName System.Speech
    $texts = @{
        2  = "Open the sales dashboard for me now."
        5  = "Please show me the total revenue for the last quarter by region and by product."
        10 = "Show me the total revenue for the last quarter by region and compare it with the same quarter of the previous year for every product line, then sort the result by growth and highlight the top five accounts."
        30 = "Good morning team. Today we will review the quarterly sales report for the western region. Revenue grew by twelve percent compared with last year, mostly because of the new enterprise contracts. However, the cost of support also went up, so the margin stayed flat. Please look at the churn numbers before our next meeting, and send me any questions about the forecast by Friday afternoon. We also need to decide on the hiring plan for the next two quarters, and the budget for the new data platform."
    }
    $rates = @{ 2 = 2; 5 = -2; 10 = 1; 30 = 0 }
    foreach ($n in 2, 5, 10, 30) {
        Set-Content -Path (Join-Path $A "ref_$n.txt") -Value $texts[$n] -Encoding ascii
        $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
        $s.Rate = $rates[$n]
        $fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
        $s.SetOutputToWaveFile((Join-Path $A "clip_${n}s.wav"), $fmt)
        $s.Speak($texts[$n])
        $s.Dispose()
    }
}

Write-Host "== 4. build (vendored, hash-checked sherpa-onnx archive)"
$env:SHERPA_ONNX_ARCHIVE_DIR = Join-Path $Work "libs"
Push-Location $Here
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { Pop-Location; Write-Host "build failed (see docs/spikes/SP-04.md, Windows notes)"; exit 2 }
$env:SP04_MODEL_DIR = $M; $env:SP04_CLIP = Join-Path $A "clip_10s.wav"; $env:SP04_CLIP_REF = Join-Path $A "ref_10.txt"
cargo test --release --locked
if ($LASTEXITCODE -ne 0) { Pop-Location; exit 2 }
Pop-Location
$Bin = Join-Path $Here "target\release\sp04-bench.exe"
$Out = Join-Path $Work "out"

Write-Host "== 5. model load, fresh process each time"
$loads = foreach ($i in 1..5) { & $Bin load --model-dir $M --threads $Threads }
$loads | Tee-Object -FilePath (Join-Path $Out "load.txt")

Write-Host "== 6. bench, threads = P-cores ($Threads)"
& $Bin bench --model-dir $M --audio-dir $A --threads $Threads --runs $Runs --platform windows | Tee-Object -FilePath (Join-Path $Out "bench-p.txt")
$rc = $LASTEXITCODE
Write-Host "== 7. bench, threads = all physical cores ($all), for the thread sensitivity"
& $Bin bench --model-dir $M --audio-dir $A --threads $all --runs $Runs --platform windows --long-secs 0 | Tee-Object -FilePath (Join-Path $Out "bench-all.txt")
Write-Host "outputs: $Out (paste load.txt, bench-p.txt and bench-all.txt into the issue)"
Write-Host "pass rule: the 'SP04_RESULT' line of bench-p.txt is PASS and every load.txt load_ms <= 3000"
exit $rc
