//! Native Rust copy of stable native/batch.go collectInputs and generated-file exclusion.
//! Source queue operations never modify 3MF inputs, slice, or print.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_SOURCE_FILES: usize = 200;
// Wails native/app.go fileInfo rejects empty 3MF files and files above 250 MiB.
const MAX_SOURCE_BYTES: u64 = 250 * 1024 * 1024;
const LEGACY_PREFIXES: &[&str] = &[
    "U1-",
    "C5-",
    "CC2-",
    "QIDI-",
    "ORCA-",
    "PRUSA-",
    "Creator5-",
    "CentauriCarbon2-",
    "XPlus5-",
];

fn is_3mf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

fn is_regular_sibling(path: &Path, name: &str) -> bool {
    if file_name(path) == Some(name) {
        return false;
    }
    let Some(folder) = path.parent() else {
        return false;
    };
    fs::symlink_metadata(folder.join(name))
        .map(|entry| entry.file_type().is_file())
        .unwrap_or(false)
}

// Conservative: never classify a user-authored printer-named 3MF as generated
// unless its original source sibling is still in the same directory.
fn has_source_sibling(output: &Path, source_name: &str) -> bool {
    if source_name.is_empty() || file_name(output) == Some(source_name) {
        return false;
    }
    let source = Path::new(source_name);
    if !is_3mf(source) {
        return false;
    }
    if is_regular_sibling(output, source_name) {
        return true;
    }
    let Some(stem) = source.file_stem().and_then(|part| part.to_str()) else {
        return false;
    };
    let Some((stem, suffix)) = stem.rsplit_once('-') else {
        return false;
    };
    if (1..=3).contains(&suffix.len()) && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        let extension = source.extension().unwrap().to_string_lossy();
        return is_regular_sibling(output, &format!("{stem}.{extension}"));
    }
    false
}

fn is_generated_sibling_output(path: &Path) -> bool {
    if !is_3mf(path) {
        return false;
    }
    let Some(name) = file_name(path) else {
        return false;
    };
    let upper = name.to_ascii_uppercase();
    // These two legacy format prefixes have always been excluded on folder import.
    if upper.starts_with("PRUSA-") || upper.starts_with("ORCA-") {
        return true;
    }
    // Stable installed profile marker:
    // <alphanumeric-model>-<0.2|0.25|0.4|0.6|0.8|Unknown>-SB<10 hex>-<source>.3mf
    if let Some((model, remainder)) = name.split_once('-') {
        if (1..=40).contains(&model.len()) && model.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            if let Some((nozzle, remainder)) = remainder.split_once('-') {
                let valid_nozzle = matches!(
                    nozzle.to_ascii_lowercase().as_str(),
                    "0.2" | "0.25" | "0.4" | "0.6" | "0.8" | "unknown"
                );
                if valid_nozzle {
                    if let Some((marker, source_name)) = remainder.split_once('-') {
                        if marker.len() == 12
                            && marker
                                .get(..2)
                                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SB"))
                            && marker
                                .get(2..)
                                .is_some_and(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
                            && has_source_sibling(path, source_name)
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    for prefix in LEGACY_PREFIXES {
        if name
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        {
            if let Some(source_name) = name.get(prefix.len()..) {
                if has_source_sibling(path, source_name) {
                    return true;
                }
            }
        }
    }
    false
}

// Go filepath.Abs removes lexical parent components before deduplicating.
// Do not use fs::canonicalize: its symlink resolution changes stable paths.
fn lexical_absolute(path: &Path) -> PathBuf {
    debug_assert!(path.is_absolute());
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                cleaned.pop();
            }
            other => cleaned.push(other.as_os_str()),
        }
    }
    cleaned
}

pub(super) fn collect_source_files(inputs: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if inputs.len() > MAX_SOURCE_FILES {
        return Err(format!("too many dropped items (limit {MAX_SOURCE_FILES})"));
    }
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    // A selected root directory is traversed, but hidden child directories are not.
    let mut pending: Vec<(PathBuf, bool)> =
        inputs.into_iter().rev().map(|path| (path, false)).collect();
    while let Some((path, from_folder)) = pending.pop() {
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            if from_folder {
                continue;
            }
            return Err(format!(
                "symlink inputs are not accepted: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            if from_folder && file_name(&path).is_some_and(|name| name.starts_with('.')) {
                continue;
            }
            let mut children = fs::read_dir(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?
                .map(|entry| {
                    entry
                        .map(|item| item.path())
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            children.sort();
            pending.extend(children.into_iter().rev().map(|child| (child, true)));
            continue;
        }
        if !metadata.is_file() || !is_3mf(&path) {
            if from_folder {
                continue;
            }
            return Err(format!("only .3mf files may be added: {}", path.display()));
        }
        if from_folder && is_generated_sibling_output(&path) {
            continue;
        }
        // Match Wails fileInfo for both folder imports and direct selection.
        // A regular .3mf extension alone does not make an empty or oversized file queueable.
        if metadata.len() == 0 || metadata.len() > MAX_SOURCE_BYTES {
            return Err(format!(
                "source must be a regular .3mf file up to 250 MB: {}",
                path.display()
            ));
        }
        // Go filepath.Abs keeps symbolic parent spellings (e.g. /var on macOS).
        // Do not canonicalize returned source paths or silently rewrite user paths.
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(&path)
        };
        let absolute = lexical_absolute(&absolute);
        let key = if cfg!(windows) {
            absolute.to_string_lossy().to_lowercase()
        } else {
            absolute.to_string_lossy().into_owned()
        };
        if seen.insert(key) {
            files.push(absolute);
            if files.len() > MAX_SOURCE_FILES {
                return Err(format!(
                    "You can queue a maximum of {MAX_SOURCE_FILES} 3MF files"
                ));
            }
        }
    }
    files.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture(name: &str) -> PathBuf {
        static ID: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sb-queue-parity-{}-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed),
            name
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folder_sources_are_recursive_sorted_filtered_and_deduplicated() {
        let dir = fixture("recursive");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::create_dir_all(dir.join(".private")).unwrap();
        let first = dir.join("first.3MF");
        let second = dir.join("nested").join("second.3mf");
        fs::write(&first, b"fixture").unwrap();
        fs::write(&second, b"fixture").unwrap();
        fs::write(dir.join(".private").join("secret.3mf"), b"skip").unwrap();
        fs::write(dir.join("notes.txt"), b"skip").unwrap();
        let items = collect_source_files(vec![dir.clone(), first.clone()]).unwrap();
        assert_eq!(items, vec![first, second]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn duplicate_parent_paths_match_go_filepath_abs() {
        let dir = fixture("parent-dedup");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let source = dir.join("source.3mf");
        fs::write(&source, b"synthetic 3MF path fixture").unwrap();
        let alias = dir.join("nested").join("..").join("source.3mf");
        // Both spellings point to one original file, not two queued conversions.
        let files = collect_source_files(vec![alias, source.clone()]).unwrap();
        assert_eq!(files, vec![source]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_outputs_are_excluded_only_for_folder_import() {
        let dir = fixture("generated");
        let source = dir.join("source.3mf");
        let legacy = dir.join("U1-source.3mf");
        let installed = dir.join("Snapmaker-0.4-SB012345abcd-source.3mf");
        let format_output = dir.join("PRUSA-userfile.3mf");
        let user_named = dir.join("QIDI-original-custom.3mf");
        for path in [&source, &legacy, &installed, &format_output, &user_named] {
            fs::write(path, b"fixture").unwrap();
        }
        let from_folder = collect_source_files(vec![dir.clone()]).unwrap();
        assert_eq!(from_folder.len(), 2);
        assert!(from_folder.contains(&source));
        assert!(from_folder.contains(&user_named));
        assert!(!from_folder.contains(&legacy));
        assert!(!from_folder.contains(&installed));
        assert!(!from_folder.contains(&format_output));
        assert_eq!(
            collect_source_files(vec![legacy.clone()]).unwrap(),
            vec![legacy]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_direct_input_is_rejected_and_limit_matches_main() {
        let dir = fixture("invalid");
        let wrong = dir.join("not-a-project.txt");
        fs::write(&wrong, b"fixture").unwrap();
        let error = collect_source_files(vec![wrong.clone()]).unwrap_err();
        assert!(error.contains("only .3mf"));
        assert!(collect_source_files(vec![wrong; 201]).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_size_gate_matches_wails_file_info() {
        let dir = fixture("size-gate");
        let empty = dir.join("empty.3mf");
        let largest = dir.join("largest.3mf");
        let oversized = dir.join("oversized.3mf");
        fs::write(&empty, b"").unwrap();
        // Sparse size-boundary fixtures avoid writing hundreds of MiB.
        fs::File::create(&largest)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES)
            .unwrap();
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();

        assert!(collect_source_files(vec![empty.clone()])
            .unwrap_err()
            .contains("up to 250 MB"));
        assert!(collect_source_files(vec![oversized.clone()])
            .unwrap_err()
            .contains("up to 250 MB"));
        assert_eq!(
            collect_source_files(vec![largest.clone()]).unwrap(),
            vec![largest.clone()]
        );
        // The same size check applies recursively during folder import.
        assert!(collect_source_files(vec![dir.clone()])
            .unwrap_err()
            .contains("up to 250 MB"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected_as_inputs_but_skipped_inside_folders() {
        use std::os::unix::fs::symlink;
        let dir = fixture("symlink");
        let source = dir.join("original.3mf");
        let linked = dir.join("link.3mf");
        fs::write(&source, b"fixture").unwrap();
        symlink(&source, &linked).unwrap();
        assert!(collect_source_files(vec![linked]).is_err());
        assert_eq!(
            collect_source_files(vec![dir.clone()]).unwrap(),
            vec![source]
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
