use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub directory: bool,
    pub sha256: String,
    pub bytes: u64,
    pub executable: bool,
}
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub fn file_hash(path: &Path, limit: u64) -> Result<String> {
    file_hash_abort(path, limit, &AtomicBool::new(false))
}

struct Interruptible<'a, R>(R, &'a AtomicBool);
impl<R: Read> Read for Interruptible<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.1.load(Ordering::SeqCst) {
            // Interrupted is retried automatically by io::copy; Other terminates it.
            return Err(std::io::Error::other("snapshot processing aborted"));
        }
        self.0.read(bytes)
    }
}

fn file_hash_abort(path: &Path, limit: u64, abort: &AtomicBool) -> Result<String> {
    let f = File::open(path)?;
    ensure!(
        f.metadata()?.len() <= limit && f.metadata()?.is_file(),
        "file is not regular or exceeds limit"
    );
    let mut hasher = Sha256::new();
    let copied = std::io::copy(&mut Interruptible(f.take(limit + 1), abort), &mut hasher)?;
    ensure!(copied <= limit, "file grew beyond limit");
    Ok(hex::encode(hasher.finalize()))
}
pub fn tree_hash(files: &BTreeMap<String, FileRecord>) -> Result<String> {
    Ok(hash(&serde_json::to_vec(files)?))
}
pub fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn ensure_directory(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        let m = fs::symlink_metadata(p).context("missing or inaccessible directory")?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "symlink/non-directory in artifact path"
        );
    }
    Ok(())
}
pub fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure_directory(path.parent().context("missing parent")?)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.file_type().is_file() && m.len() <= limit,
        "expected a bounded regular file"
    );
    let mut result = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut result)?;
    ensure!(result.len() as u64 <= limit, "file grew beyond limit");
    Ok(result)
}

/// Accept one portable relative path. No interpretation by a shell or tar extractor.
pub fn safe_path(raw: &[u8]) -> Result<PathBuf> {
    let raw = std::str::from_utf8(raw).context("non-UTF-8 archive path")?;
    ensure!(
        raw.len() <= 1024 && !raw.is_empty(),
        "empty/long archive path"
    );
    ensure!(
        !raw.chars().any(|c| c.is_control()) && !raw.contains(['\\', ':']),
        "unsafe archive filename"
    );
    let s = raw.strip_prefix("./").unwrap_or(raw).trim_end_matches('/');
    if s.is_empty() || s == "." {
        return Ok(PathBuf::new());
    }
    ensure!(
        !s.starts_with('/') && !s.contains("//"),
        "absolute or ambiguous archive path"
    );
    for part in s.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && !part.ends_with([' ', '.']),
            "unsafe path component"
        );
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            ![
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9"
            ]
            .contains(&base.as_str()),
            "non-portable reserved filename"
        );
        // Unicode normalization/case aliases differ across filesystems.
        ensure!(
            part.is_ascii(),
            "export filenames must be ASCII (file contents may be UTF-8)"
        );
    }
    let path = PathBuf::from(s);
    ensure!(
        path.components().all(|p| matches!(p, Component::Normal(_))),
        "non-relative archive path"
    );
    Ok(path)
}
pub fn excluded(path: &Path) -> bool {
    path.components().any(|p| {
        let n = p.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            n.as_str(),
            ".git"
                | "target"
                | ".claude"
                | ".codex"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".gnupg"
                | ".bench"
                | "auth.json"
                | ".credentials.json"
                | ".env"
        ) || (n.starts_with(".env.") && n != ".env.example")
    })
}
fn validate_header(header: &tar::Header) -> Result<()> {
    let kind = header.entry_type();
    ensure!(
        kind.is_file() || kind.is_dir(),
        "export rejects links, special files, extensions, and sparse entries"
    );
    ensure!(
        header.mode()? & !0o777 == 0,
        "special permission bits in export"
    );
    ensure!(
        header.link_name_bytes().is_none(),
        "link target on ordinary archive entry"
    );
    Ok(())
}

/// Extract into a new private staging directory, publish only after full validation.
/// `raw(true)` makes PAX/GNU extension headers visible so they cannot override checked names.
pub fn extract<R: Read>(
    reader: R,
    destination: &Path,
    max_bytes: u64,
    max_files: usize,
) -> Result<BTreeMap<String, FileRecord>> {
    ensure!(
        !destination.exists(),
        "refusing to replace an archived solution"
    );
    let parent = destination.parent().context("missing export parent")?;
    ensure_directory(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".export-")
        .tempdir_in(parent)?;
    let wire_limit = max_bytes
        .checked_add((max_files as u64 + 10) * 2048)
        .context("export limit overflow")?;
    let mut archive = tar::Archive::new(reader.take(wire_limit + 1));
    let mut seen = HashSet::new();
    let mut count = 0;
    let mut total = 0u64;
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        count += 1;
        ensure!(count <= max_files, "export entry count exceeded");
        validate_header(entry.header())?;
        let path = safe_path(&entry.path_bytes())?;
        let size = entry.header().size()?;
        total = total.checked_add(size).context("export size overflow")?;
        ensure!(total <= max_bytes, "export byte limit exceeded");
        if path.as_os_str().is_empty() {
            ensure!(
                entry.header().entry_type().is_dir(),
                "root entry is not a directory"
            );
            continue;
        }
        ensure!(
            seen.insert(path.to_string_lossy().to_ascii_lowercase()),
            "duplicate or case-colliding archive path"
        );
        // Validate types, sizes, and paths even for excluded entries.
        if excluded(&path) {
            continue;
        }
        let target = stage.path().join(path);
        if entry.header().entry_type().is_dir() {
            ensure!(size == 0, "directory carries data");
            fs::create_dir_all(&target)?;
        } else {
            fs::create_dir_all(target.parent().context("missing parent")?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            ensure!(
                std::io::copy(&mut entry, &mut file)? == size,
                "truncated export entry"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &target,
                    fs::Permissions::from_mode(if entry.header().mode()? & 0o111 != 0 {
                        0o755
                    } else {
                        0o644
                    }),
                )?;
            }
        }
    }
    // A transport must end at tar EOF; drain padding under the same bound.
    let mut tail = archive.into_inner();
    let mut buf = [0u8; 8192];
    while let n @ 1.. = tail.read(&mut buf)? {
        ensure!(
            buf[..n].iter().all(|b| *b == 0),
            "trailing non-padding data"
        );
    }
    ensure!(tail.limit() > 0, "archive wire-size limit exceeded");
    let files = inventory(stage.path(), max_bytes, max_files)?;
    fs::rename(stage.path(), destination)?;
    Ok(files)
}

pub fn inventory(
    root: &Path,
    max_bytes: u64,
    max_files: usize,
) -> Result<BTreeMap<String, FileRecord>> {
    ensure_directory(root)?;
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for (count, entry) in WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .enumerate()
    {
        ensure!(count < max_files, "source file count exceeded");
        let entry = entry?;
        ensure!(
            entry.file_type().is_file() || entry.file_type().is_dir(),
            "source has a link or special file"
        );
        let rel = entry.path().strip_prefix(root)?;
        let name = rel
            .to_str()
            .context("non-UTF8 path")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        safe_path(name.as_bytes())?;
        ensure!(
            seen.insert(name.to_ascii_lowercase()),
            "case-colliding source paths"
        );
        if entry.file_type().is_dir() {
            files.insert(
                name,
                FileRecord {
                    directory: true,
                    sha256: hash(b""),
                    bytes: 0,
                    executable: false,
                },
            );
            continue;
        }
        let metadata = entry.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(metadata.nlink() == 1, "hard-linked source file");
        }
        let size = metadata.len();
        total = total.checked_add(size).context("size overflow")?;
        ensure!(total <= max_bytes, "source export too large");
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        files.insert(
            name,
            FileRecord {
                directory: false,
                sha256: file_hash(entry.path(), max_bytes)?,
                bytes: size,
                executable,
            },
        );
    }
    Ok(files)
}
pub fn make_readonly(root: &Path) -> Result<()> {
    for entry in WalkDir::new(root).contents_first(true).follow_links(false) {
        let entry = entry?;
        let mut permissions = entry.metadata()?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(entry.path(), permissions)?;
    }
    Ok(())
}
pub fn pack(root: &Path, output: &Path, max_bytes: u64, max_files: usize) -> Result<()> {
    let files = inventory(root, max_bytes, max_files)?;
    let out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut tar = tar::Builder::new(out);
    for (name, record) in files {
        let mut header = tar::Header::new_ustar();
        header.set_path(&name)?;
        header.set_size(record.bytes);
        header.set_mode(if record.executable || record.directory {
            0o755
        } else {
            0o644
        });
        header.set_entry_type(if record.directory {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        if record.directory {
            tar.append(&header, std::io::empty())?;
        } else {
            tar.append(&header, File::open(root.join(name))?)?;
        }
    }
    tar.finish()?;
    tar.into_inner()?.sync_all()?;
    Ok(())
}

/// Read Docker-save/OCI-layout snapshots without loading or executing the image.
/// sbx 0.45 emits manifest.json with gzip-compressed, digest-addressed blobs.
/// The transport and aggregate decompressed layers each have a separate bound of
/// `max_image`. Only one layer is decompressed on disk at a time.
pub fn extract_image_workspace(
    image: &Path,
    prefix: &str,
    output: &Path,
    max_image: u64,
    max_bytes: u64,
    max_files: usize,
) -> Result<BTreeMap<String, FileRecord>> {
    extract_image_workspace_abort(
        image,
        prefix,
        output,
        max_image,
        max_bytes,
        max_files,
        &AtomicBool::new(false),
    )
}

pub fn extract_image_workspace_abort(
    image: &Path,
    prefix: &str,
    output: &Path,
    max_image: u64,
    max_bytes: u64,
    max_files: usize,
    abort: &AtomicBool,
) -> Result<BTreeMap<String, FileRecord>> {
    ensure!(!abort.load(Ordering::SeqCst), "snapshot processing aborted");
    ensure!(
        ["workspace", "replay"].contains(&prefix),
        "invalid snapshot source prefix"
    );
    ensure!(!output.exists(), "refusing to replace an archived solution");
    ensure!(
        fs::metadata(image)?.len() <= max_image,
        "snapshot exceeds transport limit"
    );
    let tmp = tempfile::tempdir_in(image.parent().context("snapshot parent")?)?;
    let mut manifest = None;
    let mut stored = HashSet::new();
    let mut image_bytes = 0u64;
    let mut ar = tar::Archive::new(Interruptible(File::open(image)?, abort));
    for (count, e) in ar.entries()?.raw(true).enumerate() {
        ensure!(count < 2000, "too many image entries");
        let mut e = e?;
        let path = safe_path(&e.path_bytes())?;
        validate_header(e.header())?;
        if e.header().entry_type().is_dir() {
            continue;
        }
        image_bytes = image_bytes
            .checked_add(e.size())
            .context("snapshot overflow")?;
        ensure!(image_bytes <= max_image, "snapshot unpacked size limit");
        ensure!(stored.insert(path.clone()), "duplicate image entry");
        if path == Path::new("manifest.json") {
            ensure!(e.size() <= 1024 * 1024, "oversized image manifest");
            let mut b = Vec::new();
            e.read_to_end(&mut b)?;
            manifest = Some(b);
        } else if path.extension().is_some_and(|s| s == "tar") || blob_digest(&path).is_some() {
            let dest = tmp.path().join(&path);
            fs::create_dir_all(dest.parent().context("layer parent")?)?;
            let size = e.size();
            let written = std::io::copy(
                &mut e,
                &mut OpenOptions::new().write(true).create_new(true).open(dest)?,
            )?;
            ensure!(written == size, "truncated image blob");
            if let Some(digest) = blob_digest(&path) {
                ensure!(
                    file_hash_abort(&tmp.path().join(&path), max_image, abort)? == digest,
                    "image blob digest mismatch"
                );
            }
        }
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &manifest
            .context("snapshot is not a verified Docker-save archive (missing manifest.json)")?,
    )?;
    let manifests = manifest.as_array().context("invalid image manifest")?;
    ensure!(manifests.len() == 1, "ambiguous image snapshot");
    let layers = manifests[0]["Layers"]
        .as_array()
        .context("image has no layers")?;
    ensure!(
        !layers.is_empty() && layers.len() <= 128,
        "invalid layer count"
    );
    let merged = tmp.path().join("merged");
    fs::create_dir(&merged)?;
    let mut total = 0u64;
    let mut entries = 0;
    let mut expanded = 0u64;
    for layer in layers {
        let name = safe_path(layer.as_str().context("invalid layer name")?.as_bytes())?;
        ensure!(stored.contains(&name), "missing image layer");
        let compressed = tmp.path().join(name);
        let layer_path = tmp.path().join("expanded-layer.tar");
        let mut input = File::open(&compressed)?;
        let mut magic = [0; 2];
        input.read_exact(&mut magic)?;
        input.seek(SeekFrom::Start(0))?;
        let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] {
            Box::new(flate2::read::MultiGzDecoder::new(input))
        } else {
            Box::new(input)
        };
        let mut reader = Interruptible(reader.take(max_image - expanded + 1), abort);
        let size = std::io::copy(&mut reader, &mut File::create(&layer_path)?)?;
        expanded = expanded
            .checked_add(size)
            .context("expanded image overflow")?;
        ensure!(expanded <= max_image, "decompressed image limit exceeded");
        apply_whiteouts(&layer_path, prefix, &merged, max_files, abort)?;
        let mut ar = tar::Archive::new(Interruptible(File::open(&layer_path)?, abort));
        let mut seen = BTreeMap::new();
        let mut selection = LayerSelection::default();
        for (count, e) in ar.entries()?.raw(true).enumerate() {
            ensure!(count < 1_000_000, "image layer entry limit");
            let mut e = e?;
            let Some(rel) = selection.path(&mut e, prefix)? else {
                continue;
            };
            entries += 1;
            ensure!(entries <= max_files, "layer entry count limit");
            if excluded(&rel) {
                continue;
            }
            validate_header(e.header())?;
            if rel.as_os_str().is_empty() {
                ensure!(
                    e.header().entry_type().is_dir(),
                    "workspace root changed type"
                );
                continue;
            }
            let directory = e.header().entry_type().is_dir();
            let key = rel.to_string_lossy().to_ascii_lowercase();
            if let Some((prior, was_directory)) = seen.insert(key, (rel.clone(), directory)) {
                // sbx's snapshotter repeats a directory around an opaque whiteout.
                ensure!(
                    directory && was_directory && prior == rel && e.size() == 0,
                    "duplicate or case-colliding layer path"
                );
            }
            total = total.checked_add(e.size()).context("layer size overflow")?;
            ensure!(total <= max_bytes, "source layer size limit");
            let dest = merged.join(&rel);
            let name = rel
                .file_name()
                .context("missing layer filename")?
                .to_string_lossy();
            if name.starts_with(".wh.") {
                continue; // Whiteouts were applied to prior layers before this pass.
            } else if e.header().entry_type().is_dir() {
                ensure!(e.size() == 0, "directory carries data");
                fs::create_dir_all(dest)?;
            } else {
                fs::create_dir_all(dest.parent().context("layer parent")?)?;
                if dest.is_dir() {
                    bail!("ambiguous layer directory replacement");
                }
                let size = e.size();
                ensure!(
                    std::io::copy(&mut e, &mut File::create(&dest)?)? == size,
                    "truncated source layer entry"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        dest,
                        fs::Permissions::from_mode(if e.header().mode()? & 0o111 != 0 {
                            0o755
                        } else {
                            0o644
                        }),
                    )?;
                }
            }
        }
        selection.finish()?;
        fs::remove_file(layer_path)?;
    }
    // Re-encode just the selected ordinary files and use the same export validator.
    let transport = tmp.path().join("source.tar");
    pack(&merged, &transport, max_bytes, max_files)?;
    extract(
        Interruptible(File::open(transport)?, abort),
        output,
        max_bytes,
        max_files,
    )
}

fn blob_digest(path: &Path) -> Option<&str> {
    let value = path.to_str()?.strip_prefix("blobs/sha256/")?;
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

/// Native image layers contain Linux names and PAX metadata unrelated to the
/// submission. Read bounded path overrides so they cannot disguise a workspace
/// entry. No extended metadata is ever accepted for an exported entry.
#[derive(Default)]
struct LayerSelection {
    override_path: Option<Vec<u8>>,
    extensions: usize,
}
impl LayerSelection {
    fn path<R: Read>(
        &mut self,
        entry: &mut tar::Entry<'_, R>,
        prefix: &str,
    ) -> Result<Option<PathBuf>> {
        let kind = entry.header().entry_type();
        ensure!(
            !kind.is_pax_global_extensions() && !kind.is_gnu_sparse(),
            "global or sparse image metadata is unsupported"
        );
        if kind.is_pax_local_extensions() || kind.is_gnu_longname() || kind.is_gnu_longlink() {
            self.extensions += 1;
            ensure!(
                self.extensions <= 3 && entry.size() <= 65536,
                "oversized or repeated image metadata"
            );
            if kind.is_pax_local_extensions() {
                for field in entry
                    .pax_extensions()?
                    .context("invalid image PAX header")?
                {
                    let field = field?;
                    ensure!(
                        field.key_bytes() != b"size"
                            && !field.key_bytes().starts_with(b"GNU.sparse"),
                        "PAX size or sparse override is unsupported"
                    );
                    if field.key_bytes() == b"path" {
                        ensure!(
                            self.override_path.is_none(),
                            "ambiguous image path override"
                        );
                        self.override_path = Some(field.value_bytes().to_vec());
                    }
                }
            } else if kind.is_gnu_longname() {
                ensure!(
                    self.override_path.is_none(),
                    "ambiguous image path override"
                );
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                if bytes.last() == Some(&0) {
                    bytes.pop();
                }
                self.override_path = Some(bytes);
            }
            return Ok(None);
        }
        let extended = std::mem::take(&mut self.extensions) != 0;
        let bytes = self
            .override_path
            .take()
            .unwrap_or_else(|| entry.path_bytes().into_owned());
        let raw = std::str::from_utf8(&bytes).context("non-UTF-8 image path")?;
        let raw = raw.strip_prefix("./").unwrap_or(raw).trim_end_matches('/');
        ensure!(
            !raw.starts_with('/')
                && !raw.contains(['\\', '\0'])
                && !raw.split('/').any(|p| p == ".."),
            "unsafe image layer path"
        );
        let relative = if raw == prefix {
            Some("")
        } else {
            raw.strip_prefix(prefix).and_then(|p| p.strip_prefix('/'))
        };
        let Some(relative) = relative else {
            return Ok(None);
        };
        let path = if relative.is_empty() {
            PathBuf::new()
        } else {
            safe_path(relative.as_bytes())?
        };
        // Rust's incremental build cache uses PAX paths and hard links. It is
        // already excluded, but resolve and validate its effective path first
        // so an override cannot disguise a real source entry as cache data.
        ensure!(
            !extended || excluded(&path),
            "extended metadata on exported source"
        );
        Ok(Some(path))
    }
    fn finish(self) -> Result<()> {
        ensure!(
            self.extensions == 0,
            "trailing image metadata without an entry"
        );
        Ok(())
    }
}

// OCI/Docker whiteouts remove only LOWER layer content, regardless of entry order.
fn apply_whiteouts(
    layer: &Path,
    prefix: &str,
    merged: &Path,
    max_files: usize,
    abort: &AtomicBool,
) -> Result<()> {
    let mut ar = tar::Archive::new(Interruptible(File::open(layer)?, abort));
    let mut count = 0usize;
    let mut selection = LayerSelection::default();
    for (entry_count, e) in ar.entries()?.raw(true).enumerate() {
        ensure!(entry_count < 1_000_000, "image layer entry limit");
        let mut e = e?;
        let Some(rel) = selection.path(&mut e, prefix)? else {
            continue;
        };
        count += 1;
        ensure!(count <= max_files, "layer entry count limit");
        if excluded(&rel) {
            continue;
        }
        let Some(name) = rel.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(deleted) = name.strip_prefix(".wh.") else {
            continue;
        };
        validate_header(e.header())?;
        ensure!(
            e.size() == 0 && e.header().entry_type().is_file(),
            "invalid whiteout"
        );
        let dest = merged.join(&rel);
        let parent = dest.parent().context("whiteout parent")?;
        if deleted == ".wh..opq" {
            if parent.exists() {
                fs::remove_dir_all(parent)?;
                fs::create_dir_all(parent)?;
            }
        } else {
            ensure!(
                !deleted.is_empty() && deleted != "." && deleted != "..",
                "invalid whiteout path"
            );
            let deleted = parent.join(deleted);
            if deleted.is_dir() {
                fs::remove_dir_all(deleted)?;
            } else if deleted.exists() {
                fs::remove_file(deleted)?;
            }
        }
    }
    selection.finish()
}
