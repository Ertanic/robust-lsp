# About

The robust-lsp server provides a universal LSP interface to improve the experience of working with a project using Robust Toolbox.

Inspired by Robust YAML. However, the disadvantage of the latter is that it works exclusively in VS Code. The idea behind robust-lsp is that once you install it, you can use it wherever there is support for the LSP protocol. From VS Code and Raider to Emacs and Zed.

![vscode-rider](https://github.com/user-attachments/assets/132d1ebb-bbcc-40ae-bd3e-67df4f9fe434)

## Capabilities

It is open to suggestion for extensions/improvements to the functionality.

* Code completion:
    * Prototypes
        * Fields
        * Parents (I forgot to parse interfaces in the C# codebase, so the `parent` field will not be prompted, sorry :3)
    * Components
        * Fields
        * `Icon` and `Sprite` components have code completion for rsi in the `sprite` and `state` fields.
    * Field types:
        * ProtoId
        * EntProtoId
        * bool
* Moving on to the definition:
    * Prototype in C# code
    * Prototype parent in yaml files

> [!NOTE]
> The server is under development, so features are subject to change.

# Installation

## VS Code

If you're a happy VS Code user, there's an extension [Robust LSP](https://marketplace.visualstudio.com/items?itemName=Ertanic.robust-lsp) for you that can automatically load the server, after which you can get straight to work.

> [!NOTE]
> The extension has not been tested as I am lazy, so if it refuses to load the plugin - do not throw slippers at me, I warned you.

## Other editors

First, you'll need to install the server. 

**Windows**

```powershell
iwr 'https://raw.githubusercontent.com/Ertanic/robust-lsp/refs/heads/main/scripts/install.ps1' | iex
```

**Linux**

Linux hackers, you'll figure it out on your own.

**MacOS**

You will have to build the server from the source files.

**Connect to the server**

Next, you need to find out if the editor has built-in LSP support. Let's examine the example of Rider. It doesn't have built-in support through settings, unless you create an extension (and I'm too lazy to do that). That's why we are looking for any plugin that will allow us to connect the server without any fuss. A good option is to use [lsp4ij](https://github.com/redhat-developer/lsp4ij).

Once you've installed lsp4ij, go to `File > Settings... > Languages & Frameworks > Language Servers`. Here you need to create a new server profile, to do this click on `+`.

Name the profile whatever you want, mine is `Robust LSP`. In the `Command` field you need to type `robust-lsp`. Now go to the `Mapping` tab, where in `File type` or `Language` you need to enter the following `YAML`. When you open the yaml files, the server will try to start. You can also add C# files to the triggers, it's up to your discretion.

> [!NOTE]
> I don't know if it's the server going buggy, but it's working crookedly for me in RustRover.

# Build

To build from the source files you only need rust toolchain, you can download it on the official [website](https://www.rust-lang.org/). The compiled binary will be in `target/[release|debug]/robust-lsp(.exe)`.

```bash
cargo build [--release]
```

You will need [Node.js](https://nodejs.org/en) and npm to build the VS Code plugin. The output files will be in `clients/code/out/`.

```bash
npm i
npm run compile
```

If you want to build the extension into a vsix file, you will need the [vsce](https://github.com/microsoft/vscode-vsce) manager. The output will be the file `robust-lsp-{version}.vsix`.

```bash
npm install -g @vscode/vsce
vsce package
```

But I frankly don't know why you'd want to do that, so forget it.