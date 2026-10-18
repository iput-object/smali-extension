//! Zed extension for Smali: launches `smali-lsp`.
//!
//! Binary lookup order: `lsp.smali-lsp.binary.path` setting, `smali-lsp` on PATH, then the latest
//! GitHub release for this platform (downloaded once, re-downloaded when a newer release appears).

use std::fs;

use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

const REPO: &str = "iput-object/smali-extension";
const SERVER: &str = "smali-lsp";

struct SmaliExtension {
    cached_binary: Option<String>,
}

fn asset_name(version: &str) -> Result<(String, zed::DownloadedFileType, &'static str)> {
    let (os, arch) = zed::current_platform();
    let triple = match (os, arch) {
        (zed::Os::Linux, zed::Architecture::X8664) => "x86_64-unknown-linux-musl",
        (zed::Os::Linux, zed::Architecture::Aarch64) => "aarch64-unknown-linux-musl",
        (zed::Os::Mac, zed::Architecture::X8664) => "x86_64-apple-darwin",
        (zed::Os::Mac, zed::Architecture::Aarch64) => "aarch64-apple-darwin",
        (zed::Os::Windows, zed::Architecture::X8664) => "x86_64-pc-windows-msvc",
        (os, arch) => {
            return Err(format!(
                "smali-lsp has no prebuilt binary for {os:?} {arch:?}"
            ))
        }
    };
    let _ = version;
    Ok(match os {
        zed::Os::Windows => (
            format!("{SERVER}-{triple}.zip"),
            zed::DownloadedFileType::Zip,
            ".exe",
        ),
        _ => (
            format!("{SERVER}-{triple}.tar.gz"),
            zed::DownloadedFileType::GzipTar,
            "",
        ),
    })
}

impl SmaliExtension {
    fn binary(&mut self, id: &LanguageServerId, worktree: &zed::Worktree) -> Result<String> {
        if let Some(path) = LspSettings::for_worktree(SERVER, worktree)
            .ok()
            .and_then(|s| s.binary)
            .and_then(|b| b.path)
        {
            return Ok(path);
        }
        if let Some(path) = worktree.which(SERVER) {
            return Ok(path);
        }
        if let Some(path) = &self.cached_binary {
            if fs::metadata(path).is_ok_and(|m| m.is_file()) {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        );
        let release = match release {
            Ok(r) => r,
            Err(e) => {
                // Offline or rate-limited: fall back to any version downloaded earlier.
                if let Some(path) = newest_downloaded() {
                    return Ok(path);
                }
                return Err(format!("cannot fetch {SERVER} release: {e}"));
            }
        };
        let (asset_name, file_type, exe) = asset_name(&release.version)?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("release {} has no asset {asset_name}", release.version))?;

        let dir = format!("{SERVER}-{}", release.version);
        let path = format!("{dir}/{SERVER}{exe}");
        if !fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            zed::set_language_server_installation_status(
                id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            zed::download_file(&asset.download_url, &dir, file_type)
                .map_err(|e| format!("download of {asset_name} failed: {e}"))?;
            zed::make_file_executable(&path)?;
            // Drop older downloaded versions.
            if let Ok(entries) = fs::read_dir(".") {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("{SERVER}-")) && name != dir {
                        let _ = fs::remove_dir_all(e.path());
                    }
                }
            }
        }
        zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::None,
        );
        self.cached_binary = Some(path.clone());
        Ok(path)
    }
}

fn newest_downloaded() -> Option<String> {
    let (os, _) = zed::current_platform();
    let exe = if matches!(os, zed::Os::Windows) {
        ".exe"
    } else {
        ""
    };
    let mut dirs: Vec<String> = fs::read_dir(".")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&format!("{SERVER}-")))
        .collect();
    dirs.sort();
    let dir = dirs.pop()?;
    let path = format!("{dir}/{SERVER}{exe}");
    fs::metadata(&path).is_ok().then_some(path)
}

impl zed::Extension for SmaliExtension {
    fn new() -> Self {
        SmaliExtension {
            cached_binary: None,
        }
    }

    fn language_server_command(
        &mut self,
        id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree(SERVER, worktree).ok();
        let args = settings
            .as_ref()
            .and_then(|s| s.binary.as_ref())
            .and_then(|b| b.arguments.clone())
            .unwrap_or_default();
        Ok(zed::Command {
            command: self.binary(id, worktree)?,
            args,
            env: worktree.shell_env(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        Ok(LspSettings::for_worktree(SERVER, worktree)
            .ok()
            .and_then(|s| s.initialization_options))
    }

    fn language_server_workspace_configuration(
        &mut self,
        _id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        Ok(LspSettings::for_worktree(SERVER, worktree)
            .ok()
            .and_then(|s| s.settings))
    }
}

zed::register_extension!(SmaliExtension);
