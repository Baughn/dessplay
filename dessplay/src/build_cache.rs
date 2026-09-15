//! Prune obsolete installer build units, retaining the successful build's closure.
//!
//! Cargo's JSON stream includes fresh artifacts and cached build-script output.
//! It is the retention authority; neither age nor a byte budget can evict a live
//! unit. Unknown layouts and incomplete reports fail closed. The launcher rustc
//! wrapper associates incremental directories with units without relocating or
//! invalidating existing compiler caches.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

const INCREMENTAL_RECORD: &str = ".dessplay-incremental-";

#[derive(Deserialize)]
#[serde(tag = "reason")]
enum Message {
    #[serde(rename = "compiler-artifact")]
    Artifact {
        filenames: Vec<PathBuf>,
        target: ArtifactTarget,
    },
    #[serde(rename = "build-script-executed")]
    BuildScript { out_dir: PathBuf },
    #[serde(rename = "build-finished")]
    Finished { success: bool },
    #[serde(rename = "compiler-message")]
    Diagnostic,
}

#[derive(Deserialize)]
struct ArtifactTarget {
    name: String,
}

/// Remove obsolete release build units after a successful launcher build.
///
/// `started` is recorded *before* building. Units modified since then are also
/// retained: another launcher may have built between this report and cleanup.
/// Cargo's locks exclude builds during deletion; contention skips cleanup.
/// Unknown legacy incremental directories are retained until a compiler run
/// records their ownership. Returns the number of removed compilation units.
pub fn prune(messages: &Path, target: &Path, started: SystemTime) -> Result<usize, String> {
    prune_inner(messages, target, started).map_err(|e| e.to_string())
}

fn prune_inner(messages: &Path, target: &Path, started: SystemTime) -> io::Result<usize> {
    if fs::symlink_metadata(target)?.is_symlink()
        || fs::symlink_metadata(target.join("release"))?.is_symlink()
    {
        return Err(io::Error::other("installer target directory is a symlink"));
    }
    let release = target.join("release").canonicalize()?;
    // Old Cargo used .cargo-lock; newer versions split build/artifact locks.
    // Nonblocking acquisition avoids deadlock regardless of Cargo's lock order.
    let mut locks = Vec::new();
    for name in [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"] {
        let path = release.join(name);
        if path.exists() && fs::symlink_metadata(&path)?.is_symlink() {
            return Err(io::Error::other("Cargo lock file is a symlink"));
        }
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock().map_err(io::Error::other)?;
        locks.push(file);
    }

    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for name in [".fingerprint", "build", "deps"] {
        let directory = release.join(name);
        if !directory.exists() {
            continue;
        }
        if fs::symlink_metadata(&directory)?.is_symlink() {
            return Err(io::Error::other("Cargo build directory is a symlink"));
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if let Some(hash) = entry.file_name().to_str().and_then(unit_hash) {
                groups
                    .entry(hash.to_owned())
                    .or_default()
                    .push(entry.path());
            }
        }
    }
    let mut live = HashSet::new();
    let mut owners = BTreeMap::new();
    let mut finished = false;
    let mut artifacts = 0;
    for line in BufReader::new(File::open(messages)?).lines() {
        let line = line?;
        // Procedural macros can print non-JSON lines to Cargo's stdout.
        if !line.starts_with('{') {
            continue;
        }
        if finished {
            return Err(io::Error::other("data after build-finished"));
        }
        match serde_json::from_str::<Message>(&line).map_err(io::Error::other)? {
            Message::Artifact { filenames, target } => {
                artifacts += 1;
                let mut found = false;
                let mut units = HashSet::new();
                for path in &filenames {
                    found |= mark(path, &release, &mut units)?;
                }
                if !found {
                    // Cargo uplifts binaries to release/name and its JSON omits
                    // the hashed deps/name-HASH copy. Uplifting preserves mtime
                    // on both hardlink and copy/clonefile paths. Verify contents
                    // too; never guess a hash just from the executable's name.
                    for path in &filenames {
                        let path = local_path(path, &release)?;
                        let metadata = fs::metadata(&path)?;
                        if !metadata.is_file() {
                            continue;
                        }
                        let mut digest = None;
                        for entry in fs::read_dir(release.join("deps"))? {
                            let entry = entry?;
                            let candidate = entry.metadata()?;
                            if entry.file_type()?.is_symlink()
                                || !candidate.is_file()
                                || candidate.len() != metadata.len()
                                || candidate.modified()? != metadata.modified()?
                            {
                                continue;
                            }
                            let expected = match digest {
                                Some(d) => d,
                                None => {
                                    let d = file_digest(&path)?;
                                    digest = Some(d);
                                    d
                                }
                            };
                            if file_digest(&entry.path())? == expected {
                                found |= mark(&entry.path(), &release, &mut units)?;
                            }
                        }
                    }
                }
                if !found {
                    return Err(io::Error::other("cannot identify a current Cargo artifact"));
                }
                for hash in units {
                    owners.insert(hash.clone(), target.name.replace('-', "_"));
                    live.insert(hash);
                }
            }
            Message::BuildScript { out_dir } => {
                if !mark(&out_dir, &release, &mut live)? {
                    return Err(io::Error::other("unknown Cargo build-script layout"));
                }
            }
            Message::Finished { success: true } => finished = true,
            Message::Finished { success: false } => return Err(io::Error::other("build failed")),
            Message::Diagnostic => {}
        }
    }
    if !finished || artifacts == 0 || live.is_empty() {
        return Err(io::Error::other("incomplete Cargo build report"));
    }
    if live.iter().any(|hash| !groups.contains_key(hash)) {
        return Err(io::Error::other("current build artifacts disappeared"));
    }
    verify_closure(&release, &live)?;

    // Construct the entire plan before deleting anything. Records use NUL
    // separators, so quoted paths, whitespace and newlines round-trip exactly.
    let mut incremental = BTreeMap::new();
    let mut protected_incremental = HashSet::new();
    for (hash, paths) in &groups {
        let mut directories = HashSet::new();
        for path in paths {
            let record_name = format!("{INCREMENTAL_RECORD}{hash}");
            let record = if path.file_name().and_then(|s| s.to_str()) == Some(&record_name) {
                path.clone()
            } else if path.is_dir() {
                path.join(&record_name)
            } else {
                continue;
            };
            if record.exists() {
                if fs::symlink_metadata(&record)?.is_symlink() {
                    return Err(io::Error::other(
                        "incremental ownership record is a symlink",
                    ));
                }
                for raw in fs::read(record)?
                    .split(|byte| *byte == 0)
                    .filter(|raw| !raw.is_empty())
                {
                    let path = record_path(raw)?;
                    if !path.exists() {
                        continue;
                    }
                    let path = local_path(&path, &release)?;
                    if path.parent() != Some(release.join("incremental").as_path())
                        || !path.is_dir()
                    {
                        return Err(io::Error::other("invalid incremental ownership record"));
                    }
                    directories.insert(path);
                }
            }
        }
        // A concurrent rustc invocation of the same crate can appear in more
        // than one record. Any retained unit protects all of its directories.
        if live.contains(hash)
            || paths
                .iter()
                .any(|p| !older_tree(p, started).unwrap_or(false))
        {
            live.insert(hash.clone());
            protected_incremental.extend(directories.iter().cloned());
        }
        incremental.insert(hash.clone(), directories);
    }
    // A legacy fresh artifact may not have been observed by our wrapper yet.
    // Protect its crate's directories, including directories also named in an
    // obsolete unit's record. Missing a record is never proof of disuse.
    if release.join("incremental").is_dir() {
        for (hash, name) in owners {
            if incremental.get(&hash).is_none_or(HashSet::is_empty) {
                for entry in fs::read_dir(release.join("incremental"))? {
                    let entry = entry?;
                    if entry
                        .file_name()
                        .to_str()
                        .is_some_and(|s| s.starts_with(&format!("{name}-")))
                    {
                        protected_incremental.insert(entry.path());
                    }
                }
            }
        }
    }
    let mut plan = Vec::new();
    for (hash, mut paths) in groups {
        if live.contains(&hash) {
            continue;
        }
        if let Some(directories) = incremental.get(&hash) {
            paths.extend(
                directories
                    .iter()
                    .filter(|p| !protected_incremental.contains(*p))
                    .cloned(),
            );
        }
        if paths
            .iter()
            .all(|p| older_tree(p, started).unwrap_or(false))
        {
            plan.push(paths);
        }
    }
    let removed = plan.len();
    for paths in plan {
        for path in paths {
            // Shared obsolete incremental dirs may already have been removed.
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
                Ok(_) => fs::remove_file(path)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(removed)
}

fn unit_hash(name: &str) -> Option<&str> {
    let stem = name
        .strip_prefix(INCREMENTAL_RECORD)
        .unwrap_or(name)
        .split('.')
        .next()?;
    let hash = stem.rsplit('-').next()?;
    (hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

// Cargo can uplift a *fresh*, old binary between the build report and cleanup.
// Timestamps alone cannot detect that. Check the actual fingerprints' edges:
// every dependency of the retained binary must also be retained. If a later
// build changed the graph, or Cargo changed its format, skip the whole sweep.
fn verify_closure(release: &Path, live: &HashSet<String>) -> io::Result<()> {
    #[derive(Deserialize)]
    struct Fingerprint {
        deps: Vec<(u64, String, bool, u64)>,
    }
    let mut provided = HashSet::new();
    let mut required = HashSet::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(release.join(".fingerprint"))? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(hash) = name.to_str().and_then(unit_hash) else {
            continue;
        };
        if !live.contains(hash) {
            continue;
        }
        for file in fs::read_dir(entry.path())? {
            let file = file?.path();
            if file.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let fingerprint: Fingerprint =
                serde_json::from_slice(&fs::read(&file)?).map_err(io::Error::other)?;
            let value = fs::read_to_string(file.with_extension(""))?;
            if value.len() != 16 {
                return Err(io::Error::other("unknown Cargo fingerprint format"));
            }
            provided.insert(
                u64::from_str_radix(&value, 16)
                    .map_err(io::Error::other)?
                    .swap_bytes(),
            );
            required.extend(fingerprint.deps.into_iter().map(|(_, _, _, value)| value));
            seen.insert(hash.to_owned());
        }
    }
    if !required.is_subset(&provided) || !live.is_subset(&seen) {
        return Err(io::Error::other(
            "build report no longer describes a complete dependency graph",
        ));
    }
    Ok(())
}

fn local_path(path: &Path, release: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.starts_with(release) {
        return Err(io::Error::other("artifact outside installer release cache"));
    }
    Ok(path)
}

fn mark(path: &Path, release: &Path, live: &mut HashSet<String>) -> io::Result<bool> {
    let path = local_path(path, release)?;
    let relative = path.strip_prefix(release).map_err(io::Error::other)?;
    let components: Vec<_> = relative.iter().collect();
    let candidate = match components.as_slice() {
        [directory, file] if *directory == "deps" => file.to_str().and_then(unit_hash),
        [directory, unit, ..] if *directory == "build" => unit.to_str().and_then(unit_hash),
        [_] => None,
        _ => return Err(io::Error::other("unknown Cargo artifact layout")),
    };
    if let Some(hash) = candidate {
        live.insert(hash.to_owned());
        return Ok(true);
    }
    Ok(false)
}

fn older_tree(path: &Path, started: SystemTime) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() || metadata.modified()? >= started {
        return Ok(false);
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            if !older_tree(&entry?.path(), started)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn file_digest(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest.finalize().into())
}

#[cfg(unix)]
fn record_path(raw: &[u8]) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(raw)))
}

#[cfg(not(unix))]
fn record_path(raw: &[u8]) -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::str::from_utf8(raw).map_err(io::Error::other)?,
    ))
}
