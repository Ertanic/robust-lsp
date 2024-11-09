$GithubDownloadUrl = 'https://github.com/Ertanic/robust-lsp/releases/latest/download/robust-lsp.exe'
$OutputFolder = "$env:USERPROFILE\.robust-lsp"
$OutputFilename = "robust-lsp.exe"

try {
    New-Item -ItemType Directory -Path $OutputFolder -Force | Out-Null
    Invoke-WebRequest -Uri $GithubDownloadUrl -OutFile "$OutputFolder\$OutputFilename"
    
    $PATH = [Environment]::GetEnvironmentVariable("PATH")
    [Environment]::SetEnvironmentVariable("PATH", "$PATH;$OutputFolder", "User")

    Write-Output "Robust LSP installed to $OutputFolder"
}
catch {
    Write-Error "Failed to install Robust LSP: $_"
}