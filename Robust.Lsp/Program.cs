using OmniSharp.Extensions.LanguageServer.Protocol.Models;
using OmniSharp.Extensions.LanguageServer.Protocol.Server;
using OmniSharp.Extensions.LanguageServer.Server;

var server = await LanguageServer.From(options =>
{
    options
        .WithInput(Console.OpenStandardInput())
        .WithOutput(Console.OpenStandardOutput())
        .OnInitialize(Initialize);
});

Task Initialize(ILanguageServer languageServer, InitializeParams request, CancellationToken cancellationToken)
{
    return Task.CompletedTask;
}