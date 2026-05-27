use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::types::{ActionRecord, MutationRecord, Snapshot};

const SNAPSHOT_INTERVAL: u64 = 50;
const MAX_SNAPSHOTS: usize = 50;

#[derive(Debug, Default)]
pub struct MutationReport {
    pub records: Vec<MutationRecord>,
    pub diff: String,
}

#[derive(Debug, Clone)]
pub struct RecordLogEntry {
    pub seq: u64,
    pub ts: String,
    pub op: String,
    pub path: String,
    pub dst: Option<String>,
    pub is_head: bool,
}

pub struct Record {
    project_root: PathBuf,
    record_dir: PathBuf,
    seq: u64,
    /// In-memory tree state: path -> blob hash (tracks current project state)
    tree: HashMap<String, String>,
    /// Stat cache: path -> (mtime_ns, file_size).
    /// Used by scan_project_tree to skip re-reading + re-hashing files whose
    /// stat info hasn't changed since the last scan.  Purely in-memory —
    /// rebuilt from scratch on daemon restart (which does a full baseline
    /// scan anyway).  Snapshot format is unchanged.
    file_stats: HashMap<String, (i64, u64)>,
}

impl Record {
    pub fn open(project_root: &Path) -> io::Result<Self> {
        let project_root = project_root.to_path_buf();
        let record_dir = project_root.join(".arcana/git_record");
        fs::create_dir_all(record_dir.join("objects"))?;
        fs::create_dir_all(record_dir.join("snapshots"))?;

        let head_path = record_dir.join("HEAD");
        let seq = if head_path.exists() {
            fs::read_to_string(&head_path)?
                .trim()
                .parse::<u64>()
                .unwrap_or(0)
        } else {
            0
        };

        let tree = Self::rebuild_tree(&record_dir, seq)?;
        let record = Self {
            project_root,
            record_dir,
            seq,
            tree,
            file_stats: HashMap::new(),
        };

        if !head_path.exists() {
            // Defer the baseline full-project scan to the first mutation.
            // Writing HEAD=0 with an empty tree immediately lets the daemon
            // accept connections; with_recorded_mutations / exec_with_recording
            // will perform the first scan_project_tree() lazily and take a
            // snapshot afterwards (via append's SNAPSHOT_INTERVAL trigger).
            fs::write(record.record_dir.join("HEAD"), "0")?;
        }

        Ok(record)
    }

    pub fn recover(project_root: &Path, target_seq: Option<u64>) -> io::Result<u64> {
        let mut record = Self::open(project_root)?;
        let (seq, _) = record.recover_to(target_seq)?;
        Ok(seq)
    }

    pub fn log(project_root: &Path) -> io::Result<Vec<RecordLogEntry>> {
        let record = Self::open(project_root)?;
        let mut entries = vec![RecordLogEntry {
            seq: 0,
            ts: "baseline".to_string(),
            op: "baseline".to_string(),
            path: ".".to_string(),
            dst: None,
            is_head: record.seq == 0,
        }];

        let actions_path = record.record_dir.join("actions.jsonl");
        if actions_path.exists() {
            let content = fs::read_to_string(&actions_path)?;
            for line in content.lines() {
                if let Ok(action) = serde_json::from_str::<ActionRecord>(line) {
                    entries.push(RecordLogEntry {
                        seq: action.seq,
                        ts: action.ts,
                        op: action.op,
                        path: action.path,
                        dst: action.dst,
                        is_head: action.seq == record.seq,
                    });
                }
            }
        }

        entries.sort_by_key(|entry| entry.seq);
        Ok(entries)
    }

    pub fn recover_to(&mut self, target_seq: Option<u64>) -> io::Result<(u64, MutationReport)> {
        let target_seq = target_seq.unwrap_or_else(|| self.seq.saturating_sub(1));
        if target_seq > self.seq {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "cannot recover to seq {target_seq}; latest recorded seq is {}",
                    self.seq
                ),
            ));
        }
        let target_tree = Self::rebuild_tree(&self.record_dir, target_seq)?;
        let current_tree = self.scan_project_tree()?;

        for path in sorted_removed_paths(&current_tree, &target_tree) {
            let full_path = self.project_root.join(&path);
            if full_path.exists() {
                fs::remove_file(&full_path)?;
                remove_empty_parent_dirs(&self.project_root, full_path.parent())?;
            }
        }

        for path in sorted_tree_paths(&target_tree) {
            let hash = target_tree.get(&path).expect("path came from tree");
            let content = self.read_blob(hash)?;
            let full_path = self.project_root.join(&path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(full_path, content)?;
        }

        let recovered_tree = self.scan_project_tree()?;
        let report = self.record_project_delta(current_tree, recovered_tree)?;
        Ok((target_seq, report))
    }

    /// Store content as a blob, return its sha256 hex hash.
    pub fn store_blob(&self, content: &[u8]) -> io::Result<String> {
        let hash = hex_sha256(content);
        let dir = self.record_dir.join("objects").join(&hash[..2]);
        fs::create_dir_all(&dir)?;
        let blob_path = dir.join(&hash[2..]);
        if !blob_path.exists() {
            fs::write(&blob_path, content)?;
        }
        Ok(hash)
    }

    /// Read a blob by hash.
    pub fn read_blob(&self, hash: &str) -> io::Result<Vec<u8>> {
        let path = self
            .record_dir
            .join("objects")
            .join(&hash[..2])
            .join(&hash[2..]);
        fs::read(path)
    }

    /// Scan the recoverable project tree and store every file content as a blob.
    /// Uses the in-memory stat cache to skip unchanged files — O(changed), not O(all).
    /// On the very first call (seq == 0, no baseline yet) performs the full
    /// project scan and writes the initial snapshot so subsequent daemon starts
    /// are instant.
    pub fn scan_project_tree(&mut self) -> io::Result<HashMap<String, String>> {
        let is_baseline = self.seq == 0 && self.tree.is_empty();
        if is_baseline {
            eprintln!(
                "[Arcana] Recording system initial baseline scan starting — \
                 this may take a while for large projects (one-time only)."
            );
        }
        let mut tree = HashMap::new();
        self.scan_dir(&self.project_root.clone(), &mut tree)?;
        if is_baseline {
            self.tree = tree.clone();
            self.take_snapshot()?;
            eprintln!(
                "[Arcana] Baseline snapshot complete ({} paths tracked).  \
                 Subsequent mutations will be incremental.",
                self.tree.len()
            );
        }
        Ok(tree)
    }

    pub fn record_project_delta(
        &mut self,
        before: HashMap<String, String>,
        after: HashMap<String, String>,
    ) -> io::Result<MutationReport> {
        let changed_paths = changed_paths(&before, &after);
        if changed_paths.is_empty() {
            self.tree = after;
            return Ok(MutationReport::default());
        }

        let diff = self.git_compatible_diff(&changed_paths, &before, &after)?;
        self.tree = before.clone();

        let mut records = Vec::new();
        for path in changed_paths {
            let prev_blob = before.get(&path).cloned();
            let blob = after.get(&path).cloned();
            let op = if blob.is_some() { "write" } else { "delete" };
            let seq = self.append(op, &path, blob, prev_blob, None)?;
            records.push(MutationRecord {
                seq,
                op: op.to_string(),
                path,
            });
        }

        Ok(MutationReport { records, diff })
    }

    /// Append an action record. Triggers snapshot if needed. Returns new seq.
    pub fn append(
        &mut self,
        op: &str,
        path: &str,
        blob: Option<String>,
        prev_blob: Option<String>,
        dst: Option<String>,
    ) -> io::Result<u64> {
        self.seq += 1;

        match op {
            "write" => {
                if let Some(ref b) = blob {
                    self.tree.insert(path.to_string(), b.clone());
                }
            }
            "delete" => {
                self.tree.remove(path);
            }
            "rename" => {
                if let Some(b) = self.tree.remove(path) {
                    if let Some(ref d) = dst {
                        self.tree.insert(d.clone(), b);
                    }
                }
            }
            _ => {}
        }

        let record = ActionRecord {
            seq: self.seq,
            ts: now_iso8601(),
            op: op.to_string(),
            path: path.to_string(),
            blob,
            prev_blob,
            dst,
        };

        let log_path = self.record_dir.join("actions.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let line =
            serde_json::to_string(&record).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(file, "{}", line)?;
        file.sync_all()?;

        fs::write(self.record_dir.join("HEAD"), self.seq.to_string())?;

        if self.seq % SNAPSHOT_INTERVAL == 0 {
            self.take_snapshot()?;
        }

        Ok(self.seq)
    }

    fn scan_dir(&mut self, dir: &Path, tree: &mut HashMap<String, String>) -> io::Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let rel = match path.strip_prefix(&self.project_root) {
                Ok(rel) => rel,
                Err(_) => continue,
            };
            if should_skip_path(rel) {
                continue;
            }

            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                self.scan_dir(&path, tree)?;
            } else if file_type.is_file() {
                let rel_str = normalize_rel_path(rel);
                // Stat the file.  If mtime + size match the in-memory cache,
                // reuse the stored hash — no disk read, no SHA-256.
                let meta = path.metadata()?;
                let cur_mtime = file_modified_ns(&meta);
                let cur_size = meta.len();
                if let Some(&(cached_mtime, cached_size)) = self.file_stats.get(&rel_str) {
                    if cached_mtime == cur_mtime && cached_size == cur_size {
                        if let Some(hash) = self.tree.get(&rel_str) {
                            let hash = hash.clone();
                            tree.insert(rel_str.clone(), hash);
                            // Update stat cache with fresh values (no-op if unchanged)
                            self.file_stats.insert(rel_str, (cur_mtime, cur_size));
                            continue; // ← skip read + hash — O(1) instead of O(file_size)
                        }
                    }
                }
                // File is new or changed — read, hash, store.
                let content = fs::read(&path)?;
                let hash = self.store_blob(&content)?;
                tree.insert(rel_str.clone(), hash);
                // Cache the stat info for the next incremental scan
                self.file_stats.insert(rel_str, (cur_mtime, cur_size));
            }
        }
        Ok(())
    }

    fn git_compatible_diff(
        &self,
        paths: &[String],
        before: &HashMap<String, String>,
        after: &HashMap<String, String>,
    ) -> io::Result<String> {
        let diff_dir = self
            .project_root
            .join(".arcana/tmp/record_diff")
            .join(std::process::id().to_string());
        if diff_dir.exists() {
            let _ = fs::remove_dir_all(&diff_dir);
        }
        fs::create_dir_all(&diff_dir)?;

        let mut combined = String::new();
        for (idx, path) in paths.iter().enumerate() {
            let old_path = if let Some(hash) = before.get(path) {
                let file = diff_dir.join(format!("{idx}.old"));
                fs::write(&file, self.read_blob(hash)?)?;
                file
            } else {
                PathBuf::from("/dev/null")
            };
            let new_path = if let Some(hash) = after.get(path) {
                let file = diff_dir.join(format!("{idx}.new"));
                fs::write(&file, self.read_blob(hash)?)?;
                file
            } else {
                PathBuf::from("/dev/null")
            };

            let output = Command::new("git")
                .arg("diff")
                .arg("--no-index")
                .arg(&old_path)
                .arg(&new_path)
                .current_dir(&self.project_root)
                .output();

            let chunk = match output {
                Ok(output) => {
                    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                    if text.is_empty() && !output.stderr.is_empty() {
                        text = String::from_utf8_lossy(&output.stderr).into_owned();
                    }
                    normalize_diff_paths(text, path, &old_path, &new_path)
                }
                Err(_) => fallback_unified_diff(path, before.get(path), after.get(path), self)?,
            };

            if !chunk.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(chunk.trim_end_matches('\n'));
                combined.push('\n');
            }
        }

        let _ = fs::remove_dir_all(&diff_dir);
        Ok(combined)
    }

    fn take_snapshot(&self) -> io::Result<()> {
        let snap = Snapshot {
            seq: self.seq,
            ts: now_iso8601(),
            tree: self.tree.clone(),
        };
        let filename = format!("{:06}.json", self.seq);
        let snap_path = self.record_dir.join("snapshots").join(&filename);
        let json = serde_json::to_string_pretty(&snap)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(&snap_path, json)?;

        self.prune_snapshots()?;
        Ok(())
    }

    fn prune_snapshots(&self) -> io::Result<()> {
        let snap_dir = self.record_dir.join("snapshots");
        let mut entries: Vec<_> = fs::read_dir(&snap_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        if entries.len() > MAX_SNAPSHOTS {
            let to_remove = entries.len() - MAX_SNAPSHOTS;
            for entry in &entries[..to_remove] {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    /// Rebuild tree state from latest snapshot at or before `current_seq`.
    fn rebuild_tree(record_dir: &Path, current_seq: u64) -> io::Result<HashMap<String, String>> {
        let snap_dir = record_dir.join("snapshots");
        let mut tree = HashMap::new();
        let mut replay_from = 0u64;

        if snap_dir.exists() {
            let mut snapshots = Vec::new();
            for entry in fs::read_dir(&snap_dir)? {
                let entry = entry?;
                if !entry.path().extension().is_some_and(|ext| ext == "json") {
                    continue;
                }
                let data = fs::read_to_string(entry.path())?;
                if let Ok(snap) = serde_json::from_str::<Snapshot>(&data) {
                    if snap.seq <= current_seq {
                        snapshots.push(snap);
                    }
                }
            }
            snapshots.sort_by_key(|snap| snap.seq);
            if let Some(snap) = snapshots.pop() {
                tree = snap.tree;
                replay_from = snap.seq;
            }
        }

        let actions_path = record_dir.join("actions.jsonl");
        if actions_path.exists() {
            let content = fs::read_to_string(&actions_path)?;
            for line in content.lines() {
                if let Ok(rec) = serde_json::from_str::<ActionRecord>(line) {
                    if rec.seq <= replay_from {
                        continue;
                    }
                    if rec.seq > current_seq {
                        break;
                    }
                    match rec.op.as_str() {
                        "write" => {
                            if let Some(b) = rec.blob {
                                tree.insert(rec.path, b);
                            }
                        }
                        "delete" => {
                            tree.remove(&rec.path);
                        }
                        "rename" => {
                            if let Some(b) = tree.remove(&rec.path) {
                                if let Some(d) = rec.dst {
                                    tree.insert(d, b);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(tree)
    }
}

fn changed_paths(before: &HashMap<String, String>, after: &HashMap<String, String>) -> Vec<String> {
    let mut paths = BTreeSet::new();
    paths.extend(before.keys().cloned());
    paths.extend(after.keys().cloned());
    paths
        .into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

fn sorted_tree_paths(tree: &HashMap<String, String>) -> Vec<String> {
    let mut paths: Vec<String> = tree.keys().cloned().collect();
    paths.sort();
    paths
}

fn sorted_removed_paths(
    current: &HashMap<String, String>,
    target: &HashMap<String, String>,
) -> Vec<String> {
    let mut paths: Vec<String> = current
        .keys()
        .filter(|path| !target.contains_key(*path))
        .cloned()
        .collect();
    paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    paths
}

fn should_skip_path(rel: &Path) -> bool {
    let path = normalize_rel_path(rel);
    path == ".git"
        || path.starts_with(".git/")
        || path == ".arcana/git_record"
        || path.starts_with(".arcana/git_record/")
        || path == ".arcana/authority.sock"
        || path == ".arcana/authorized_prompt.md"
        || path == ".arcana/web_cache"
        || path.starts_with(".arcana/web_cache/")
        || path == ".arcana/tmp"
        || path.starts_with(".arcana/tmp/")
}

fn normalize_rel_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn remove_empty_parent_dirs(project_root: &Path, parent: Option<&Path>) -> io::Result<()> {
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent == project_root
        || should_skip_path(parent.strip_prefix(project_root).unwrap_or(parent))
    {
        return Ok(());
    }
    if parent.read_dir()?.next().is_none() {
        fs::remove_dir(parent)?;
        remove_empty_parent_dirs(project_root, parent.parent())?;
    }
    Ok(())
}

fn fallback_unified_diff(
    path: &str,
    before: Option<&String>,
    after: Option<&String>,
    record: &Record,
) -> io::Result<String> {
    let mut out = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    if let Some(hash) = before {
        out.push_str(&String::from_utf8_lossy(&record.read_blob(hash)?));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if let Some(hash) = after {
        out.push_str(&String::from_utf8_lossy(&record.read_blob(hash)?));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

fn normalize_diff_paths(
    text: String,
    project_path: &str,
    old_path: &Path,
    new_path: &Path,
) -> String {
    let old_label = if old_path == Path::new("/dev/null") {
        "/dev/null".to_string()
    } else {
        format!("a/{project_path}")
    };
    let new_label = if new_path == Path::new("/dev/null") {
        "/dev/null".to_string()
    } else {
        format!("b/{project_path}")
    };

    let lines: Vec<&str> = text.lines().collect();
    let body_start = lines.iter().position(|line| line.starts_with("@@"));
    let mut out = format!("diff --git a/{project_path} b/{project_path}\n");
    out.push_str(&format!("--- {old_label}\n+++ {new_label}\n"));
    if let Some(start) = body_start {
        for line in &lines[start..] {
            out.push_str(line);
            out.push('\n');
        }
    } else {
        for line in lines {
            if line.starts_with("diff --git ")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Nanosecond-resolution modified time from file metadata.
/// Falls back to 0 on platforms that don't provide it.
fn file_modified_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}s_since_epoch", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arcana_record_test_{}_{}_{}",
            name,
            std::process::id(),
            now_iso8601()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn baseline_recovery_restores_existing_file() {
        let root = temp_project("baseline");
        fs::write(root.join("README.md"), "original\n").unwrap();

        let _record = Record::open(&root).unwrap();
        fs::remove_file(root.join("README.md")).unwrap();

        Record::recover(&root, Some(0)).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "original\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tree_delta_records_shell_style_mutations_and_recovers() {
        let root = temp_project("delta");
        fs::write(root.join("a.txt"), "one\n").unwrap();

        let mut record = Record::open(&root).unwrap();
        let before = record.scan_project_tree().unwrap();
        fs::write(root.join("a.txt"), "two\n").unwrap();
        fs::write(root.join("b.txt"), "new\n").unwrap();
        let after = record.scan_project_tree().unwrap();
        let report = record.record_project_delta(before, after).unwrap();
        assert_eq!(report.records.len(), 2);

        fs::remove_file(root.join("a.txt")).unwrap();
        fs::remove_file(root.join("b.txt")).unwrap();
        Record::recover(&root, Some(2)).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "two\n");
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "new\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_preserves_later_recorded_states() {
        let root = temp_project("history");
        fs::write(root.join("a.txt"), "one\n").unwrap();

        let mut record = Record::open(&root).unwrap();
        let before = record.scan_project_tree().unwrap();
        fs::write(root.join("a.txt"), "two\n").unwrap();
        fs::write(root.join("b.txt"), "new\n").unwrap();
        let after = record.scan_project_tree().unwrap();
        record.record_project_delta(before, after).unwrap();

        Record::recover(&root, Some(0)).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "one\n");
        assert!(!root.join("b.txt").exists());

        Record::recover(&root, Some(2)).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "two\n");
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "new\n");
        let _ = fs::remove_dir_all(root);
    }
}
