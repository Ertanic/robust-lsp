$GithubDownloadUrl = 'https://github.com/Ertanic/robust-lsp/releases/latest/download/robust-lsp-win-x86_64.exe'
$OutputFolder = "$env:USERPROFILE\.robust-lsp"
$OutputFilename = "robust-lsp.exe"

try {
    New-Item -ItemType Directory -Path $OutputFolder -Force | Out-Null
    Invoke-WebRequest -Uri $GithubDownloadUrl -OutFile "$OutputFolder\$OutputFilename"
    
    $PATH = [Environment]::GetEnvironmentVariable("PATH")
    if ($PATH -notlike "*$OutputFolder*") {
        [Environment]::SetEnvironmentVariable("PATH", "$PATH;$OutputFolder", "User")
    }

    Write-Output "Robust LSP installed to $OutputFolder"
}
catch {
    Write-Error "Failed to install Robust LSP: $_"
}