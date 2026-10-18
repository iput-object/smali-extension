# smali-lsp

Editor support for Smali (Android Dalvik bytecode assembly). One local language server
(`smali-lsp`, Rust, LSP over stdio) plus small wrappers for Zed, VS Code, Neovim and Helix.
Everything runs locally; the only network access is the Zed extension downloading the server
binary from GitHub Releases.

## Features

- Diagnostics: syntax errors, unknown opcodes, register counts and ranges, wide register pairs,
  literal ranges, labels, control flow, unresolved classes and members
- Completion for opcodes, directives, registers, labels, types and members
- Hover, go to definition, find references, workspace symbols, call hierarchy
- Rename, quick fixes, formatting, semantic tokens
- Resolution against your workspace, `android.jar` and extra smali roots. Missing classes are
  never errors, so partial or obfuscated apps work.

## Build

    cargo build --release -p smali-lsp -p smali-check

Binaries end up in `target/release/`. Put `smali-lsp` on your `PATH` or point your editor at it.

## Editors

| Editor | Setup |
|---|---|
| Zed | Extensions > Install Dev Extension > `editors/zed`. Uses `smali-lsp` from `PATH`, the `lsp.smali-lsp.binary.path` setting, or the latest GitHub release. |
| VS Code | `editors/vscode`: `npm install && npm run compile`, then package or run it as a dev extension. |
| Neovim | Copy `editors/nvim/smali.lua` into your config. Highlighting: `:TSInstall smali`. |
| Helix | Merge `editors/helix/languages.toml` into `~/.config/helix/languages.toml`. |
| JetBrains | Use LSP4IJ with `smali-lsp` as the command. |

## Settings

Passed as `initializationOptions` or `workspace/didChangeConfiguration` under `smali`:

| Key | Meaning |
|---|---|
| `androidJar` | Path to `android.jar`. Default: newest platform in the Android SDK. |
| `apiLevel` | API level used to pick `android.jar` from the SDK. |
| `extraRoots` | Directories of reference smali (e.g. decompiled framework). |
| `cacheDir` | Index cache directory. Default: OS cache dir. |
| `diagnostics` | Severity per code: `off`, `hint`, `info`, `warning`, `error`. Example: `{ "S301": "off" }`. |

## CLI

`smali-check` runs the same analysis over directories and prints diagnostics:

    smali-check DIR... [--android-jar PATH] [--extra-root DIR] [--json] [--errors-only] [--quiet]

Run `smali-check --help` for all options.

## Layout

| Path | What |
|---|---|
| `crates/smali-syntax` | tree-sitter wrapper, typed model, descriptors, line index |
| `crates/smali-opcodes` | opcode table generated from google/smali `Opcode.java` |
| `crates/smali-index` | symbol index, resolution, cache, `android.jar` import |
| `crates/smali-analysis` | diagnostics, completion, hover, navigation, actions, rename, formatter |
| `crates/smali-lsp` | the language server |
| `crates/smali-check` | command-line checker |
| `grammar/tree-sitter-smali` | vendored tree-sitter grammar with fixes (see `VENDORED.md`) |
| `editors/` | editor wrappers |

## Development

    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

## Credits

See `CREDITS.md`.
