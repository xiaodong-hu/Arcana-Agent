use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::types::{ActionRecord, Snapshot};

const SNAPSHOT_INTERVAL: u64 = 50;
const MAX_SNAPSHOTS: usize = 50;

pub struct Record {
    record_dir: PathBuf,
    seq: u64,
    /// In-memory tree state: path -> blob hash (tracks current project state)
    tree: HashMap<String, String>,
}

impl Record {
    pub fn open(project_root: &Path) -> io::Result<Self> {
        let record_dir = project_root.join(".arcana/git_record");
        fs::create_dir_all(record_dir.join("objects"))?;
        fs::create_dir_all(record_dir.join("snapshots"))?;

        let head_path = record_dir.join("HEAD");
        let seq = if head_path.exists() {
            fs::read_to_string(&head_path)?.trim().parse::<u64>().unwrap_or(0)
        } else {
            0
        };

        // Rebuild tree from latest snapshot + subsequent actions
        let tree = Self::rebuild_tree(&record_dir, seq)?;

        Ok(Self { record_dir, seq, tree })
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
        let path = self.record_dir.join("objects").join(&hash[..2]).join(&hash[2..]);
        fs::read(path)
    }

    /// Get the blob hash for an existing file (None if file doesn't exist).
    pub fn hash_file(&self, path: &Path) -> io::Result<Option<String>> {
        if path.exists() {
            let content = fs::read(path)?;
            Ok(Some(hex_sha256(&content)))
        } else {
            Ok(None)
        }
    }

    /// Append an action record. Triggers snapshot if needed. Returns new seq.
    pub fn append(&mut self, op: &str, path: &str, blob: Option<String>, prev_blob: Option<String>, dst: Option<String>) -> io::Result<u64> {
        self.seq += 1;

        // Update in-memory tree
        match op {
            "write" => { if let Some(ref b) = blob { self.tree.insert(path.to_string(), b.clone()); } }
            "delete" => { self.tree.remove(path); }
            "rename" => {
                if let Some(b) = self.tree.remove(path) {
                    if let Some(ref d) = dst { self.tree.insert(d.clone(), b); }
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
        let mut file = OpenOptions::new().create(true).append(true).open(&log_path)?;
        let line = serde_json::to_string(&record).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(file, "{}", line)?;
        file.sync_all()?;

        fs::write(self.record_dir.join("HEAD"), self.seq.to_string())?;

        // Snapshot every N mutations
        if self.seq % SNAPSHOT_INTERVAL == 0 {
            self.take_snapshot()?;
        }

        Ok(self.seq)
    }

    fn take_snapshot(&self) -> io::Result<()> {
        let snap = Snapshot {
            seq: self.seq,
            ts: now_iso8601(),
            tree: self.tree.clone(),
        };
        let filename = format!("{:06}.json", self.seq);
        let snap_path = self.record_dir.join("snapshots").join(&filename);
        let json = serde_json::to_string_pretty(&snap).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(&snap_path, json)?;

        // Prune old snapshots beyond MAX_SNAPSHOTS
        self.prune_snapshots()?;
        Ok(())
    }

    fn prune_snapshots(&self) -> io::Result<()> {
        let snap_dir = self.record_dir.join("snapshots");
        let mut entries: Vec<_> = fs::read_dir(&snap_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
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

    /// Rebuild tree state from latest snapshot + replaying subsequent actions.
    fn rebuild_tree(record_dir: &Path, current_seq: u64) -> io::Result<HashMap<String, String>> {
        let snap_dir = record_dir.join("snapshots");
        let mut tree = HashMap::new();
        let mut replay_from = 0u64;

        // Find latest snapshot
        if snap_dir.exists() {
            let mut snaps: Vec<_> = fs::read_dir(&snap_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
                .collect();
            snaps.sort_by_key(|e| e.file_name());

            if let Some(latest) = snaps.last() {
                let data = fs::read_to_string(latest.path())?;
                if let Ok(snap) = serde_json::from_str::<Snapshot>(&data) {
                    tree = snap.tree;
                    replay_from = snap.seq;
                }
            }
        }

        // Replay actions after snapshot
        let actions_path = record_dir.join("actions.jsonl");
        if actions_path.exists() {
            let content = fs::read_to_string(&actions_path)?;
            for line in content.lines() {
                if let Ok(rec) = serde_json::from_str::<ActionRecord>(line) {
                    if rec.seq <= replay_from { continue; }
                    if rec.seq > current_seq { break; }
                    match rec.op.as_str() {
                        "write" => { if let Some(b) = rec.blob { tree.insert(rec.path, b); } }
                        "delete" => { tree.remove(&rec.path); }
                        "rename" => {
                            if let Some(b) = tree.remove(&rec.path) {
                                if let Some(d) = rec.dst { tree.insert(d, b); }
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

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{}s_since_epoch", secs)
}
