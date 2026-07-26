// libgit2 bindings for Elixir (git2ex): status / diff / stage / commit /
// log / show for building git UIs. Local repository access only —
// the crate is built without network transports. Every NIF runs on a dirty
// CPU scheduler: libgit2 walks trees and hashes files, which can take far
// longer than a normal scheduler slice on big repositories.

use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::{DiffFormat, DiffOptions, IndexAddOption, Repository, Status, StatusOptions};
use rustler::NifMap;

const MAX_DIFF_BYTES: usize = 512 * 1024;
const MAX_FILE_AT_BYTES: usize = 2 * 1024 * 1024;

#[derive(NifMap)]
struct FileStatus {
    path: String,
    status: String,
    staged: bool,
    unstaged: bool,
}

#[derive(NifMap)]
struct StatusResult {
    repo: bool,
    root: Option<String>,
    branch: Option<String>,
    files: Vec<FileStatus>,
}

#[derive(NifMap)]
struct DiffResult {
    diff: String,
    binary: bool,
    truncated: bool,
}

#[derive(NifMap)]
struct FileAtResult {
    content: String,
    binary: bool,
    truncated: bool,
    missing: bool,
}

#[derive(NifMap)]
struct CommitInfo {
    hash: String,
    author: String,
    date_unix: i64,
    subject: String,
}

#[derive(NifMap)]
struct ShowResult {
    text: String,
    truncated: bool,
}

fn git_error(err: git2::Error) -> String {
    err.message().to_string()
}

fn open(path: &str) -> Result<Repository, String> {
    Repository::discover(path).map_err(|_| format!("not a git repository: {path}"))
}

/// Largest valid-UTF-8 prefix that fits in `max` bytes.
fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

// --- status -----------------------------------------------------------------

#[rustler::nif(schedule = "DirtyCpu")]
fn status(path: String) -> Result<StatusResult, String> {
    let repo = match Repository::discover(&path) {
        Ok(repo) => repo,
        Err(_) => {
            return Ok(StatusResult {
                repo: false,
                root: None,
                branch: None,
                files: vec![],
            })
        }
    };

    let root = repo
        .workdir()
        .map(|p| p.to_string_lossy().trim_end_matches('/').to_string());

    let branch = match repo.head() {
        Ok(head) if head.is_branch() => head.shorthand().map(|s| s.to_string()),
        Ok(_) => Some("HEAD".to_string()),
        // Unborn branch: HEAD points at a ref that doesn't exist yet.
        Err(_) => unborn_branch(&repo),
    };

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        // Staged renames show as R (like `git status`), not as an A+D pair.
        // Head↔index only — worktree rename detection is quadratic on big
        // change sets and the CLI doesn't do it for status either.
        .renames_head_to_index(true);

    let statuses = repo.statuses(Some(&mut opts)).map_err(git_error)?;
    let mut files = Vec::new();

    for entry in statuses.iter() {
        // A renamed entry's `path()` is the OLD path; the panel lists files
        // under where they live NOW — take the delta's new_file when present.
        let path = entry
            .head_to_index()
            .and_then(|d| d.new_file().path())
            .or_else(|| entry.index_to_workdir().and_then(|d| d.new_file().path()))
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| entry.path().map(str::to_string));
        let Some(path) = path else { continue };

        let s = entry.status();
        if s.is_ignored() {
            continue;
        }
        let (code, staged, unstaged) = porcelain(s);
        files.push(FileStatus {
            path,
            status: code,
            staged,
            unstaged,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(StatusResult {
        repo: true,
        root,
        branch,
        files,
    })
}

fn unborn_branch(repo: &Repository) -> Option<String> {
    let head = repo.find_reference("HEAD").ok()?;
    let target = head.symbolic_target()?;
    Some(target.trim_start_matches("refs/heads/").to_string())
}

/// Porcelain `XY` status code plus staged/unstaged flags for one entry. A
/// file with both index and worktree changes (e.g. `MM`) is both, and shows
/// up in both lists like Fork does.
fn porcelain(s: Status) -> (String, bool, bool) {
    if s.contains(Status::CONFLICTED) {
        return ("UU".to_string(), false, true);
    }

    // Untracked: only a working-tree "new" bit.
    if s.contains(Status::WT_NEW) && !has_index_change(s) {
        return ("??".to_string(), false, true);
    }

    let x = if s.contains(Status::INDEX_NEW) {
        'A'
    } else if s.contains(Status::INDEX_MODIFIED) {
        'M'
    } else if s.contains(Status::INDEX_DELETED) {
        'D'
    } else if s.contains(Status::INDEX_RENAMED) {
        'R'
    } else if s.contains(Status::INDEX_TYPECHANGE) {
        'T'
    } else {
        ' '
    };

    let y = if s.contains(Status::WT_MODIFIED) {
        'M'
    } else if s.contains(Status::WT_DELETED) {
        'D'
    } else if s.contains(Status::WT_RENAMED) {
        'R'
    } else if s.contains(Status::WT_TYPECHANGE) {
        'T'
    } else {
        ' '
    };

    (format!("{x}{y}"), x != ' ', y != ' ')
}

fn has_index_change(s: Status) -> bool {
    s.intersects(
        Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    )
}

// --- diff -------------------------------------------------------------------

#[rustler::nif(schedule = "DirtyCpu")]
fn diff_file(path: String, file: String, staged: bool) -> Result<DiffResult, String> {
    let repo = open(&path)?;

    let mut opts = DiffOptions::new();
    opts.pathspec(&file)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);

    // Match the perspective each list renders against:
    //   * unstaged ("changes"): index ↔ workdir
    //   * staged:               HEAD  ↔ index
    // so an `MM` file's two diffs line up with its two rows.
    let index = repo.index().map_err(git_error)?;
    let diff = if staged {
        let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))
            .map_err(git_error)?
    } else {
        repo.diff_index_to_workdir(Some(&index), Some(&mut opts))
            .map_err(git_error)?
    };

    let (text, binary) = format_diff(&diff)?;
    let truncated = text.len() > MAX_DIFF_BYTES;

    Ok(DiffResult {
        diff: truncate_utf8(&text, MAX_DIFF_BYTES),
        binary,
        truncated,
    })
}

/// Full contents of one file at a revision (`HEAD`, a sha, `sha^`, …) for the
/// side-by-side diff view. A missing path (new/deleted file, bad rev) is
/// reported as `missing`, not an error.
#[rustler::nif(schedule = "DirtyCpu")]
fn file_at(path: String, rev: String, file: String) -> Result<FileAtResult, String> {
    let repo = open(&path)?;

    let missing = FileAtResult {
        content: String::new(),
        binary: false,
        truncated: false,
        missing: true,
    };

    // ":0" (the index) is git-CLI revision syntax that libgit2's revparse
    // does not understand — read the staged blob straight from the index.
    let blob = if rev == ":0" {
        let index = match repo.index() {
            Ok(index) => index,
            Err(_) => return Ok(missing),
        };
        match index
            .get_path(Path::new(&file), 0)
            .and_then(|entry| repo.find_blob(entry.id).ok())
        {
            Some(blob) => blob,
            None => return Ok(missing),
        }
    } else {
        let spec = format!("{rev}:{file}");
        match repo.revparse_single(&spec).and_then(|o| o.peel_to_blob()) {
            Ok(blob) => blob,
            Err(_) => return Ok(missing),
        }
    };

    if blob.is_binary() {
        return Ok(FileAtResult {
            content: String::new(),
            binary: true,
            truncated: false,
            missing: false,
        });
    }

    match std::str::from_utf8(blob.content()) {
        Ok(text) => Ok(FileAtResult {
            content: truncate_utf8(text, MAX_FILE_AT_BYTES),
            binary: false,
            truncated: text.len() > MAX_FILE_AT_BYTES,
            missing: false,
        }),
        // Not valid UTF-8: treat like binary so the caller falls back.
        Err(_) => Ok(FileAtResult {
            content: String::new(),
            binary: true,
            truncated: false,
            missing: false,
        }),
    }
}

fn format_diff(diff: &git2::Diff) -> Result<(String, bool), String> {
    let mut buf = String::new();
    let mut binary = false;

    diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        if delta.flags().is_binary() {
            binary = true;
        }
        match line.origin() {
            '+' | '-' | ' ' => buf.push(line.origin()),
            _ => {}
        }
        buf.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(git_error)?;

    Ok((buf, binary))
}

// --- staging ----------------------------------------------------------------

#[rustler::nif(schedule = "DirtyCpu")]
fn stage(path: String, file: String) -> Result<bool, String> {
    let repo = open(&path)?;
    let mut index = repo.index().map_err(git_error)?;
    // add_all mirrors `git add <pathspec>`: stages additions, modifications
    // and deletions of matching files.
    index
        .add_all([&file].iter(), IndexAddOption::DEFAULT, None)
        .map_err(git_error)?;
    index.write().map_err(git_error)?;
    Ok(true)
}

#[rustler::nif(schedule = "DirtyCpu")]
fn unstage(path: String, file: String) -> Result<bool, String> {
    let repo = open(&path)?;

    match repo.head().and_then(|h| h.peel_to_commit()) {
        Ok(head) => {
            repo.reset_default(Some(head.as_object()), [&file])
                .map_err(git_error)?;
        }
        Err(_) => {
            // No commits yet: unstaging just removes the entry from the index.
            let mut index = repo.index().map_err(git_error)?;
            index.remove_path(Path::new(&file)).map_err(git_error)?;
            index.write().map_err(git_error)?;
        }
    }

    Ok(true)
}

#[rustler::nif(schedule = "DirtyCpu")]
fn discard(path: String, file: String) -> Result<bool, String> {
    let repo = open(&path)?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let tracked = head_tree
        .as_ref()
        .map(|tree| tree.get_path(Path::new(&file)).is_ok())
        .unwrap_or(false);

    if tracked {
        let tree = head_tree.unwrap();
        let mut co = CheckoutBuilder::new();
        co.force().update_index(true).path(&file);
        repo.checkout_tree(tree.as_object(), Some(&mut co))
            .map_err(git_error)?;
    } else {
        let full = repo
            .workdir()
            .ok_or_else(|| "bare repository".to_string())?
            .join(&file);
        std::fs::remove_file(&full).map_err(|e| format!("could not delete {file}: {e}"))?;
    }

    Ok(true)
}

// --- commit -----------------------------------------------------------------

#[rustler::nif(schedule = "DirtyCpu")]
fn commit(path: String, message: String) -> Result<String, String> {
    let repo = open(&path)?;
    let sig = repo
        .signature()
        .map_err(|_| "no git identity configured (user.name / user.email)".to_string())?;

    let mut index = repo.index().map_err(git_error)?;
    let tree_oid = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_oid).map_err(git_error)?;

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    // Reject empty commits (nothing staged).
    match &parent {
        Some(p) if p.tree_id() == tree_oid => {
            return Err("nothing to commit — working tree clean".to_string())
        }
        None if tree.is_empty() => return Err("nothing to commit".to_string()),
        _ => {}
    }

    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, &message, &tree, &parents)
        .map_err(git_error)?;

    Ok(short_hash(&oid.to_string()))
}

/// Amend HEAD with the current index; an empty message keeps the original.
#[rustler::nif(schedule = "DirtyCpu")]
fn commit_amend(path: String, message: String) -> Result<String, String> {
    let repo = open(&path)?;

    let mut index = repo.index().map_err(git_error)?;
    let tree_id = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_id).map_err(git_error)?;

    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(git_error)?;

    let message = if message.trim().is_empty() {
        head.message().unwrap_or("").to_string()
    } else {
        message
    };

    let oid = head
        .amend(Some("HEAD"), None, None, None, Some(&message), Some(&tree))
        .map_err(git_error)?;

    Ok(short_hash(&oid.to_string()))
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

// --- log / show -------------------------------------------------------------

#[rustler::nif(schedule = "DirtyCpu")]
fn log(path: String, limit: usize) -> Result<Vec<CommitInfo>, String> {
    let repo = open(&path)?;

    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return Ok(vec![]),
    };

    if revwalk.push_head().is_err() {
        // Empty repository (unborn HEAD).
        return Ok(vec![]);
    }

    let mut commits = Vec::new();
    for oid in revwalk.take(limit) {
        let oid = oid.map_err(git_error)?;
        let c = repo.find_commit(oid).map_err(git_error)?;
        commits.push(CommitInfo {
            hash: short_hash(&oid.to_string()),
            author: c.author().name().unwrap_or("").to_string(),
            date_unix: c.time().seconds(),
            subject: c.summary().unwrap_or("").to_string(),
        });
    }

    Ok(commits)
}

#[rustler::nif(schedule = "DirtyCpu")]
fn show(path: String, hash: String) -> Result<ShowResult, String> {
    let repo = open(&path)?;
    let commit = repo
        .revparse_single(&hash)
        .and_then(|o| o.peel_to_commit())
        .map_err(|_| format!("no such commit: {hash}"))?;

    let mut text = String::new();
    text.push_str(&format!("commit {}\n", commit.id()));
    text.push_str(&format!(
        "Author: {} <{}>\n",
        commit.author().name().unwrap_or(""),
        commit.author().email().unwrap_or("")
    ));
    text.push('\n');
    for line in commit.message().unwrap_or("").lines() {
        text.push_str("    ");
        text.push_str(line);
        text.push('\n');
    }
    text.push('\n');

    let tree = commit.tree().map_err(git_error)?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .map_err(git_error)?;

    let (patch, _binary) = format_diff(&diff)?;
    text.push_str(&patch);

    let truncated = text.len() > MAX_DIFF_BYTES;

    Ok(ShowResult {
        text: truncate_utf8(&text, MAX_DIFF_BYTES),
        truncated,
    })
}

rustler::init!("Elixir.Git2Ex");
