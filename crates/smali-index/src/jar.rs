//! Framework import from android.jar (class files only: flags, hierarchy, member signatures).

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{ClassInfo, MemberInfo, Origin};

pub fn read_jar(path: &Path) -> anyhow::Result<Vec<ClassInfo>> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        if !entry.name().is_ok_and(|n| n.ends_with(".class")) {
            continue;
        }
        buf.clear();
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        if let Some(c) = class_from_bytes(&buf) {
            out.push(c);
        }
    }
    Ok(out)
}

fn class_from_bytes(bytes: &[u8]) -> Option<ClassInfo> {
    let mut opts = cafebabe::ParseOptions::default();
    opts.parse_bytecode(false);
    let cf = cafebabe::parse_class_with_options(bytes, &opts).ok()?;
    let desc = |n: &dyn std::fmt::Display| format!("L{n};");
    Some(ClassInfo {
        name: desc(&cf.this_class),
        super_class: cf.super_class.as_ref().map(|s| desc(s)),
        interfaces: cf.interfaces.iter().map(|i| desc(i)).collect(),
        flags: u32::from(cf.access_flags.bits()),
        fields: cf
            .fields
            .iter()
            .map(|f| MemberInfo {
                name: f.name.to_string(),
                desc: f.descriptor.to_string(),
                flags: u32::from(f.access_flags.bits()),
                span: None,
            })
            .collect(),
        methods: cf
            .methods
            .iter()
            .map(|m| MemberInfo {
                name: m.name.to_string(),
                desc: m.descriptor.to_string(),
                flags: u32::from(m.access_flags.bits()),
                span: None,
            })
            .collect(),
        origin: Origin::AndroidJar,
        path: None,
        span: None,
    })
}

/// android.jar from config: explicit path, else `<sdk>/platforms/android-<api>`, else the newest
/// platform. The SDK is `$ANDROID_HOME`, then `$ANDROID_SDK_ROOT`, then common install paths.
pub fn find_android_jar(explicit: Option<&Path>, api_level: Option<u32>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.is_file().then(|| p.to_path_buf());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut sdks: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|k| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .collect();
    if let Some(h) = &home {
        sdks.extend([
            h.join("Android/Sdk"),
            h.join("Library/Android/sdk"),
            h.join(".android"),
        ]);
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        sdks.push(PathBuf::from(l).join("Android/Sdk"));
    }
    for sdk in sdks {
        let platforms = sdk.join("platforms");
        if let Some(api) = api_level {
            let jar = platforms.join(format!("android-{api}/android.jar"));
            if jar.is_file() {
                return Some(jar);
            }
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&platforms) else {
            continue;
        };
        let newest = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let n: u32 = name.strip_prefix("android-")?.parse().ok()?;
                let jar = e.path().join("android.jar");
                jar.is_file().then_some((n, jar))
            })
            .max_by_key(|(n, _)| *n);
        if let Some((_, jar)) = newest {
            return Some(jar);
        }
    }
    None
}
