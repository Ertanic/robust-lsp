import { ExtensionContext, Uri, window, workspace } from 'vscode';
import { Executable, LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';
import { exec } from 'child_process';

let client: LanguageClient;
let info: GitHubReleasesAPIResponse | undefined;

export async function activate(context: ExtensionContext) {
	console.log('An extension "robust-lsp" is now active!');

	const robust_name = getExecutableFilename();
	const lsp_path = process.env.LSP_SERVER_PATH ? Uri.from({ path: process.env.LSP_SERVER_PATH.concat(process.platform === 'win32' ? '.exe' : ''), scheme: 'file' }) : Uri.joinPath(context.globalStorageUri, robust_name);
	console.log(`robust-lsp path: ${lsp_path.fsPath}`);
	console.log(`robust-lsp file exists: `, await fileExists(lsp_path));

	if (!await fileExists(lsp_path)) {
		let selection = await window.showInformationMessage('robust-lsp is not installed.', 'Install', 'Cancel');
		if (!selection || selection === 'Cancel') {
			deactivate();
			return;
		}

		info = await getLatestReleaseInfo();
		const url = info?.assets.find(a => a.name === robust_name)?.browser_download_url;

		if (!url) {
			await window.showInformationMessage(`Cannot find suitable version of robust-lsp for your platform ${process.platform} with arch ${process.arch}. You can check out extension Github and create issue if it's a bug`);
			deactivate();
			return;
		}

		const buffer = await (await (await f(url)).blob()).bytes();
		await workspace.fs.writeFile(lsp_path, buffer);

		window.showInformationMessage('robust-lsp has been installed.');
	}

	await exec(`${lsp_path.fsPath} --version`, async (err, stdout, stderr) => {
		if (err) {
			console.error(err);
			return;
		}

		if (stderr) {
			console.error(stderr);
		}

		console.log("robust-lsp version: " + stdout);
		if (!info) {
			info = await getLatestReleaseInfo();
		}
		console.log("robust-lsp latest version: " + info?.tag_name);

		const curr_ver = new Version(stdout);
		const latest_ver = new Version(info?.tag_name!);
		const newer = await curr_ver.isNewer(latest_ver);

		console.log(`Latest version (${info?.tag_name}) newer current version (${stdout})?: ${newer}`);
		
		if (curr_ver.isNewer(latest_ver)) {
			const selection = await window.showInformationMessage(`Current version of robust-lsp: “${stdout}”, newer version found: “${info?.tag_name}”.`, 'Update', 'Cancel');

			if (!selection || selection === 'Cancel') {
				deactivate();
				return;
			}

			if (selection === 'Update') {
				const uri = info?.assets.find(a => a.name === robust_name)?.browser_download_url;
				if (!uri) {
					deactivate();
					return;
				}

				const buffer = await (await (await f(uri)).blob()).bytes();
				await workspace.fs.writeFile(lsp_path, buffer);

				window.showInformationMessage('robust-lsp has been updated.');
			}
		}

		const command = lsp_path.fsPath;
		const run: Executable = {
			command,
			options: {
				env: {
					...process.env,
					RUST_LOG: 'trace',
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
	});
}

export function deactivate() {
	if (!client) {
		return undefined;
	}
	console.log('An extension "robust-lsp" is now inactive!');
	return client.stop();
}

async function getLatestReleaseInfo(): Promise<GitHubReleasesAPIResponse | undefined> {
	try {
		const response = await f('https://api.github.com/repos/Ertanic/robust-lsp/releases/latest');
		if (!response.ok) {
			throw new Error(`Response status: ${response.status}`);
		}

		const json = await response.json();
		return json as GitHubReleasesAPIResponse;
	} catch (error: any) {
		console.error(error.message);
	}
}

async function fileExists(path: Uri): Promise<boolean> {
	try {
		const res = await workspace.fs.stat(path);
		return res !== undefined;
	} catch {
		return false;
	}
}

function f(s: string, init?: RequestInit): Promise<Response> {
	return fetch(s, {
		headers: { 'User-Agent': 'vscode-robust-lsp' },
		...init
	});
}

function getExecutableFilename(): string {
	return `robust-lsp-${process.platform === 'win32' ? 'win-x86_64.exe' : 'linux-x86_64'}`;
}

interface GitHubReleasesAPIResponse {
	url: string;
	assets_url: string;
	upload_url: string;
	html_url: string;
	id: number;
	node_id: string;
	tag_name: string;
	target_commitish: string;
	name: string;
	draft: boolean;
	author: any;
	prerelease: boolean;
	created_at: string;
	published_at: string;
	assets: Asset[];
	tarball_url: string;
	zipball_url: string;
	body: any | null;
}

interface Asset {
	url: string;
	name: string;
	browser_download_url: string;
}

class Version {
	major: number;
	minor: number;
	patch: number;

	constructor(ver: string) {
		const [major, minor, patch] = (ver.startsWith('v') ? ver.substring(1) : ver).split('.');
		this.major = parseInt(major, 10);
		this.minor = parseInt(minor, 10);
		this.patch = parseInt(patch, 10);
	}

	isNewer(ver: Version): boolean {
		return this.major < ver.major || this.minor < ver.minor || this.patch < ver.patch
	}
}