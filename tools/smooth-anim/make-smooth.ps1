<#
.SYNOPSIS
  Build a smooth-animation mod (extra in-between frames) for actions of an .act file from YOUR game data.
  Output is derived from the original game: keep it local, never commit or ship it.
.EXAMPLE
  # cheap CPU cross-fade (x4), hero walk directions 8..15
  .\make-smooth.ps1 -Game <path-to-NewPAL> -Mode crossfade -Factor 4
.EXAMPLE
  # RIFE (GPU! run when not gaming), x4
  .\make-smooth.ps1 -Game <path-to-NewPAL> -Mode rife -Factor 4 -Rife <path-to-rife-ncnn-vulkan.exe>
#>
param(
  [Parameter(Mandatory)] [string]$Game,
  [string]$Act = 'all_char',
  [int]$Slot = 0,
  [string]$Actions = '8-15',           # '8-15' or '0,1,8'
  [ValidateSet('crossfade','rife')] [string]$Mode = 'crossfade',
  [ValidateSet(2,4,8)] [int]$Factor = 4,
  [string]$Rife = 'rife-ncnn-vulkan.exe',
  [string]$RifeModel = 'rife-v4.6',
  [string]$Work = '',                  # default: <NewPAL-re>\extracted\smooth (outside the repo)
  [string]$ModDir = ''                 # default: <NewPAL-re>\mods-test\smooth-<act>-<slot>-<mode>
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$engine = (Resolve-Path (Join-Path $here '..\..')).Path
$root = Split-Path -Parent $engine
if (-not $Work) { $Work = Join-Path $root 'extracted\smooth' }
if (-not $ModDir) { $ModDir = Join-Path $root "mods-test\smooth-$Act-$Slot-$Mode" }
$list = foreach ($p in $Actions.Split(',')) { if ($p -match '^(\d+)-(\d+)$') { [int]$matches[1]..[int]$matches[2] } else { [int]$p } }
Push-Location $engine
try {
  cargo build -q -p newpal-assets --example anim_tool
  $tool = Join-Path $engine 'target\debug\examples\anim_tool.exe'
  foreach ($a in $list) {
    $orig = Join-Path $Work "orig\$Act-$Slot-$a"
    $out  = Join-Path $ModDir "act\$Act\$Slot\$a"
    & $tool export $Game $Act $Slot $a $orig
    if ($Mode -eq 'crossfade') {
      & $tool crossfade $orig $out ($Factor - 1)
    } else {
      $w = Join-Path $Work "rife\$Act-$Slot-$a"
      Remove-Item -Recurse -Force "$w\rgb_out","$w\alpha_out" -ErrorAction SilentlyContinue
      & $tool prep-rife $orig $w
      $n = (Get-ChildItem "$w\rgb" -Filter *.png).Count            # frames incl. loop seam
      $target = ($n - 1) * $Factor + 1                              # in-betweens for every pair
      foreach ($c in 'rgb','alpha') {
        New-Item -ItemType Directory -Force "$w\${c}_out" | Out-Null
        & $Rife -i "$w\$c" -o "$w\${c}_out" -n $target -m $RifeModel -j 1:1:1
        if ($LASTEXITCODE -ne 0) { throw "rife failed on $c" }
      }
      $total = ((Get-Content "$orig\frames.json" | ConvertFrom-Json).durations_ms | Measure-Object -Sum).Sum
      & $tool merge-rife $w $out $total
    }
  }
  Write-Host "mod written: $ModDir  -> run: target\debug\newpal.exe --mods-dir $(Split-Path $ModDir)"
} finally { Pop-Location }