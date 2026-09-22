use bench::archive;
use std::{
    fs,
    io::{Cursor, Write},
};

fn tar(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut out = tar::Builder::new(Vec::new());
    for (name, kind, data) in entries {
        let mut header = tar::Header::new_ustar();
        header.set_path(name).unwrap();
        header.set_entry_type(tar::EntryType::new(*kind));
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        out.append(&header, Cursor::new(data)).unwrap();
    }
    out.into_inner().unwrap()
}
fn image(layer: &[u8]) -> Vec<u8> {
    let mut compressor = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    compressor.write_all(layer).unwrap();
    let blob = compressor.finish().unwrap();
    let name = format!("blobs/sha256/{}", archive::hash(&blob));
    let manifest = serde_json::to_vec(&serde_json::json!([{"Layers": [name]}])).unwrap();
    tar(&[("manifest.json", b'0', &manifest), (&name, b'0', &blob)])
}
fn extract(bytes: &[u8], max_image: u64) -> anyhow::Result<tempfile::TempDir> {
    let work = tempfile::tempdir()?;
    fs::write(work.path().join("snapshot.tar"), bytes)?;
    let result = archive::extract_image_workspace(
        &work.path().join("snapshot.tar"),
        "workspace",
        &work.path().join("solution"),
        max_image,
        1024 * 1024,
        100,
    );
    if result.is_err() {
        assert!(!work.path().join("solution").exists());
    }
    result?;
    Ok(work)
}

#[test]
fn observed_oci_blob_gzip_and_repeated_directory_format_exports() {
    let layer = tar(&[
        ("workspace", b'5', b""),
        ("workspace/src", b'5', b""),
        ("workspace/src/.wh..wh..opq", b'0', b""),
        ("workspace/src", b'5', b""),
        ("workspace/src/main.rs", b'0', b"unchanged"),
        ("var/lib/dpkg/pkg:amd64", b'0', b"unrelated Linux path"),
        ("home/agent/auth.json", b'0', b"private"),
        ("workspace/target/output", b'0', b"excluded"),
    ]);
    let result = extract(&image(&layer), 1024 * 1024).unwrap();
    assert_eq!(
        fs::read(result.path().join("solution/src/main.rs")).unwrap(),
        b"unchanged"
    );
    assert_eq!(
        archive::inventory(&result.path().join("solution"), 1000, 100)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn cancelled_snapshot_does_not_publish_source_and_can_be_retried_explicitly() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("snapshot.tar");
    let output = work.path().join("solution");
    fs::write(
        &input,
        image(&tar(&[("workspace/main.rs", b'0', b"untouched")])),
    )
    .unwrap();
    let abort = AtomicBool::new(true);
    let error = archive::extract_image_workspace_abort(
        &input,
        "workspace",
        &output,
        1024 * 1024,
        1024,
        10,
        &abort,
    )
    .unwrap_err();
    assert!(error.to_string().contains("aborted"));
    assert!(!output.exists());
    abort.store(false, Ordering::SeqCst);
    archive::extract_image_workspace_abort(
        &input,
        "workspace",
        &output,
        1024 * 1024,
        1024,
        10,
        &abort,
    )
    .unwrap();
    assert_eq!(fs::read(output.join("main.rs")).unwrap(), b"untouched");
}

#[test]
fn compressed_bombs_and_corrupt_blob_identities_are_rejected() {
    let layer = tar(&[("workspace/bomb", b'0', &vec![0; 256 * 1024])]);
    assert!(
        extract(&image(&layer), 20000)
            .unwrap_err()
            .to_string()
            .contains("decompressed")
    );
    let bad_name = format!("blobs/sha256/{}", "0".repeat(64));
    let manifest = serde_json::to_vec(&serde_json::json!([{"Layers": [bad_name]}])).unwrap();
    let corrupt = tar(&[
        ("manifest.json", b'0', &manifest),
        (&bad_name, b'0', &layer),
    ]);
    assert!(
        extract(&corrupt, 1024 * 1024)
            .unwrap_err()
            .to_string()
            .contains("digest")
    );
}

#[test]
fn duplicate_files_case_aliases_and_source_links_remain_rejected() {
    for layer in [
        tar(&[
            ("workspace/file", b'0', b"a"),
            ("workspace/file", b'0', b"b"),
        ]),
        tar(&[("workspace/src", b'5', b""), ("workspace/SRC", b'5', b"")]),
        tar(&[("workspace/link", b'2', b"")]),
    ] {
        assert!(extract(&image(&layer), 1024 * 1024).is_err());
    }
}

fn pax(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut n = body.len() + 1;
    while n != body.len() + n.to_string().len() {
        n = body.len() + n.to_string().len();
    }
    format!("{n}{body}").into_bytes()
}

#[test]
fn unrelated_native_metadata_is_skipped_but_cannot_disguise_source_entries() {
    let metadata = pax("mtime", "123.45");
    let unrelated = tar(&[
        ("PaxHeaders/file", b'x', &metadata),
        ("usr/lib/file", b'0', b"base"),
        ("workspace/source", b'0', b"preserved"),
    ]);
    extract(&image(&unrelated), 1024 * 1024).unwrap();
    let disguised = tar(&[
        ("PaxHeaders/file", b'x', &pax("path", "workspace/evil")),
        ("usr/lib/apparently-unrelated", b'0', b"evil"),
    ]);
    assert!(extract(&image(&disguised), 1024 * 1024).is_err());
    let gnu = tar(&[
        ("././@LongLink", b'L', b"workspace/evil\0"),
        ("usr/lib/file", b'0', b"evil"),
    ]);
    assert!(extract(&image(&gnu), 1024 * 1024).is_err());
    let huge = tar(&[("PaxHeaders/file", b'x', &vec![b'x'; 65537])]);
    assert!(extract(&image(&huge), 1024 * 1024).is_err());
}

#[test]
fn snapshot_prefix_cannot_select_host_configuration() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("snapshot.tar");
    fs::write(
        &path,
        image(&tar(&[("home/agent/auth.json", b'0', b"private")])),
    )
    .unwrap();
    for prefix in ["home", "../workspace", "", "/workspace"] {
        assert!(
            archive::extract_image_workspace(
                &path,
                prefix,
                &work.path().join("out"),
                10000,
                1000,
                100
            )
            .is_err()
        );
    }
}
