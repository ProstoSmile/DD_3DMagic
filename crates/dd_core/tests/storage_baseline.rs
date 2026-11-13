#![cfg(windows)]
use dd_core::storage::ManagedWorkspace;
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn publication_rejects_scene_or_manifest_edits_made_during_external_work() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let parent = repository.join(".work/temp");
    fs::create_dir_all(&parent).unwrap();
    let root = parent.join(format!(
        "baseline_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = ManagedWorkspace::new(&root).unwrap();
    let original = workspace
        .publication_baseline("props", "fixtures", "test_asset")
        .unwrap();
    let scene = workspace
        .final_scene("props", "fixtures", "test_asset")
        .unwrap();
    let manifest = workspace.final_manifest("test_asset").unwrap();
    fs::write(&scene, b"artist scene").unwrap();
    fs::write(&manifest, b"artist manifest").unwrap();
    let staged_scene = workspace
        .write_new(Path::new(".assetlab/staging/run/scene.blend"), b"new scene")
        .unwrap();
    let staged_manifest = workspace
        .write_new(
            Path::new(".assetlab/staging/run/asset.toml"),
            b"new manifest",
        )
        .unwrap();
    assert!(
        workspace
            .publish_scene_checked(
                "props",
                "fixtures",
                "test_asset",
                &staged_scene,
                &staged_manifest,
                &original
            )
            .is_err()
    );
    assert_eq!(fs::read(&scene).unwrap(), b"artist scene");
    assert!(staged_scene.exists());
    let baseline = workspace
        .publication_baseline("props", "fixtures", "test_asset")
        .unwrap();
    fs::write(&manifest, b"artist changed manifest").unwrap();
    assert!(
        workspace
            .publish_scene_checked(
                "props",
                "fixtures",
                "test_asset",
                &staged_scene,
                &staged_manifest,
                &baseline
            )
            .is_err()
    );
    assert_eq!(fs::read(&manifest).unwrap(), b"artist changed manifest");
    let baseline = workspace
        .publication_baseline("props", "fixtures", "test_asset")
        .unwrap();
    workspace
        .publish_scene_checked(
            "props",
            "fixtures",
            "test_asset",
            &staged_scene,
            &staged_manifest,
            &baseline,
        )
        .unwrap();
    assert_eq!(fs::read(&scene).unwrap(), b"new scene");
    let resolved = root.canonicalize().unwrap();
    assert!(resolved.starts_with(parent.canonicalize().unwrap()));
    fs::remove_dir_all(resolved).unwrap();
}
