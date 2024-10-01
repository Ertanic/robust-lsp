import { ExtensionContext, window } from 'vscode';
import { Executable, LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: ExtensionContext) {
	console.log('An extension "robust-lsp" is now active!');

	const command = process.env.LSP_SERVER_PATH || 'robust-lsp';
	console.log(`Starting robust-lsp with command: ${command}`);
	const run: Executable = {
		command,
		options: {
			env: {
				...process.env,
				RUST_LOG: 'debug',
			},
		},
	};
	const serverOptions: ServerOptions = {
		run,
		debug: run,
	};
	const clientOptions: LanguageClientOptions = {
		documentSelector: [
			{ scheme: "file", language: "csharp" },
			{ scheme: "file", language: "yaml" },
			{ scheme: "file", language: "fluent", pattern: "**/*.ftl" },
		],
	};

	client = new LanguageClient('robust-lsp', 'Robust Language Server', serverOptions, clientOptions);
	client.start();
}

export function deactivate() {
	if (!client) {
		return undefined;
	}
	return client.stop();
}