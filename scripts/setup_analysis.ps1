param([string]$Destination = "$PSScriptRoot/../target/analysis-runtime")
$ErrorActionPreference = 'Stop'
python -m venv $Destination
& "$Destination/Scripts/python.exe" -m pip install torch torchvision --index-url https://download.pytorch.org/whl/cpu
if ($LASTEXITCODE -ne 0) { throw 'PyTorch installation failed' }
& "$Destination/Scripts/python.exe" -m pip install timm safetensors huggingface_hub
if ($LASTEXITCODE -ne 0) { throw 'Analysis dependencies installation failed' }
