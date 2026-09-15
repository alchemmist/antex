use antex_keyring_store::DefaultKeyringStore;
use antex_keyring_store::KeyringStore;
use antex_utils_absolute_path::AbsolutePathBuf;
use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::fs::FileTimes;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use walkdir::WalkDir;

mod configuration;
mod credential_keys;
mod credentials;
mod databases;
mod metadata;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub source: PathBuf,
    pub source_aliases: Vec<PathBuf>,
    pub destination: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub external_symlinks: Vec<PathBuf>,
}

pub fn completed(source: &Path, destination: &Path) -> anyhow::Result<Option<MigrationPlan>> {
    let Some(plan) = completed_destination(destination)? else {
        return Ok(None);
    };
    let source = AbsolutePathBuf::relative_to_current_dir(source)?.to_path_buf();
    Ok(plan.source_aliases.contains(&source).then_some(plan))
}

pub fn completed_destination(destination: &Path) -> anyhow::Result<Option<MigrationPlan>> {
    let marker = destination.join("antex-migration.json");
    if !marker.is_file() {
        return Ok(None);
    }
    let plan: MigrationPlan = serde_json::from_slice(&fs::read(marker)?)
        .map_err(|_| anyhow::anyhow!("invalid Antex migration manifest"))?;
    if plan.destination == destination.canonicalize()? {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum Entry {
    Directory,
    File {
        bytes: u64,
        modified: SystemTime,
        sha256: String,
    },
    Symlink(PathBuf),
}

pub struct PreparedMigration {
    plan: MigrationPlan,
    source_entries: BTreeMap<PathBuf, Entry>,
    staging: tempfile::TempDir,
    credentials: credentials::KeyringMigration,
    _lock: File,
}

pub fn inspect(source: &Path, destination: &Path) -> anyhow::Result<MigrationPlan> {
    let source_input = AbsolutePathBuf::relative_to_current_dir(source)?.to_path_buf();
    let source = source_input
        .canonicalize()
        .context("read source home directory")?;
    ensure!(source.is_dir(), "source home is not a directory");
    ensure!(destination.is_absolute(), "destination must be absolute");
    let parent = destination.parent().context("destination has no parent")?;
    let destination = parent
        .canonicalize()
        .context("destination parent must exist")?
        .join(destination.file_name().context("destination has no name")?);
    ensure!(
        !destination.starts_with(&source) && !source.starts_with(&destination),
        "source and destination must be separate directory trees"
    );
    ensure!(
        destination
            .symlink_metadata()
            .is_err_and(|e| e.kind() == io::ErrorKind::NotFound),
        "destination already exists; migration never overwrites an existing home"
    );
    let entries = inventory(&source)?;
    let mut files = 0;
    let mut bytes = 0;
    let mut external_symlinks = Vec::new();
    for (relative, entry) in entries {
        match entry {
            Entry::File { bytes: size, .. } => {
                files += 1;
                bytes += size;
            }
            Entry::Symlink(target) => {
                let resolved = source
                    .join(&relative)
                    .parent()
                    .unwrap_or(&source)
                    .join(target);
                if !resolved
                    .canonicalize()
                    .is_ok_and(|p| p.starts_with(&source))
                {
                    external_symlinks.push(relative);
                }
            }
            Entry::Directory => {}
        }
    }
    Ok(MigrationPlan {
        source_aliases: if source_input == source {
            vec![source.clone()]
        } else {
            vec![source_input, source.clone()]
        },
        source,
        destination,
        files,
        bytes,
        external_symlinks,
    })
}

pub async fn prepare(plan: MigrationPlan) -> anyhow::Result<PreparedMigration> {
    prepare_with_keyring(plan, Arc::new(DefaultKeyringStore)).await
}

async fn prepare_with_keyring(
    plan: MigrationPlan,
    keyring: Arc<dyn KeyringStore>,
) -> anyhow::Result<PreparedMigration> {
    let lock_path = plan.destination.with_extension("antex-migration.lock");
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock()
        .context("another migration is using this destination")?;
    ensure!(
        inspect(&plan.source_aliases[0], &plan.destination)? == plan,
        "source home changed; inspect it again"
    );
    let source_entries = inventory(&plan.source)?;
    let staging = tempfile::Builder::new()
        .prefix(".antex-migration-")
        .tempdir_in(
            plan.destination
                .parent()
                .context("destination has no parent")?,
        )?;
    for (relative, entry) in &source_entries {
        let source = plan.source.join(relative);
        let target = staging.path().join(relative);
        match entry {
            Entry::Directory => fs::create_dir_all(&target)?,
            Entry::File { sha256, .. } => {
                fs::copy(&source, &target)?;
                ensure!(
                    file_hash(&target)? == *sha256,
                    "source changed while copying {}",
                    relative.display()
                );
                let metadata = source.metadata()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        &target,
                        fs::Permissions::from_mode(metadata.permissions().mode() | 0o200),
                    )?;
                }
                #[cfg(windows)]
                {
                    let mut writable = metadata.permissions();
                    writable.set_readonly(false);
                    fs::set_permissions(&target, writable)?;
                }
                File::options()
                    .write(true)
                    .open(&target)?
                    .set_times(FileTimes::new().set_modified(metadata.modified()?))?;
            }
            Entry::Symlink(link) => {
                let absolute = AbsolutePathBuf::resolve_path_against_base(
                    link,
                    source.parent().context("symlink has no parent")?,
                )
                .to_path_buf();
                let link = plan
                    .source_aliases
                    .iter()
                    .find_map(|alias| absolute.strip_prefix(alias).ok())
                    .map(|relative| plan.destination.join(relative))
                    .unwrap_or(absolute);
                create_symlink(&link, &target, &source)?;
            }
        }
    }
    ensure!(
        inventory(&plan.source)? == source_entries,
        "source home changed during migration; stop all source clients and retry"
    );
    for alias in &plan.source_aliases {
        databases::rewrite_paths(staging.path(), alias, &plan.destination).await?;
        configuration::rewrite(staging.path(), alias, &plan.destination)?;
    }
    metadata::rewrite(staging.path(), &plan)?;
    let credentials = credentials::KeyringMigration::prepare(
        &plan.source,
        staging.path(),
        &plan.destination,
        keyring,
    )?;
    for (relative, entry) in source_entries.iter().rev() {
        if matches!(entry, Entry::File { .. } | Entry::Directory) {
            let target = if relative == Path::new("secrets/codex_auth.age") {
                staging.path().join("secrets/antex_auth.age")
            } else {
                staging.path().join(relative)
            };
            if !target.try_exists()?
                && relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.ends_with(".sqlite-wal") || name.ends_with(".sqlite-shm")
                    })
            {
                continue;
            }
            fs::set_permissions(
                target,
                fs::metadata(plan.source.join(relative))?.permissions(),
            )?;
        }
    }
    Ok(PreparedMigration {
        plan,
        source_entries,
        staging,
        credentials,
        _lock: lock,
    })
}

impl PreparedMigration {
    pub fn staging_path(&self) -> &Path {
        self.staging.path()
    }

    pub fn publish(mut self) -> anyhow::Result<MigrationPlan> {
        self.credentials.verify()?;
        ensure!(
            inventory(&self.plan.source)? == self.source_entries,
            "source home changed before publication; stop all source clients and retry"
        );
        ensure!(
            !self.plan.destination.try_exists()?,
            "destination appeared during migration"
        );
        let manifest = self.staging.path().join("antex-migration.json");
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(manifest)?;
        serde_json::to_writer_pretty(&mut file, &self.plan)?;
        file.sync_all()?;
        fs::set_permissions(
            self.staging.path(),
            fs::metadata(&self.plan.source)?.permissions(),
        )?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            self.staging.path(),
            rustix::fs::CWD,
            &self.plan.destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        #[cfg(windows)]
        fs::rename(self.staging.path(), &self.plan.destination)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        bail!("atomic migration publication is unsupported on this platform");
        self.credentials.commit();
        Ok(self.plan)
    }
}

fn inventory(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Entry>> {
    let mut entries = BTreeMap::new();
    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path)?;
        let value = if metadata.file_type().is_symlink() {
            Entry::Symlink(fs::read_link(path)?)
        } else if metadata.is_dir() {
            Entry::Directory
        } else if metadata.is_file() {
            Entry::File {
                bytes: metadata.len(),
                modified: metadata.modified()?,
                sha256: file_hash(path)?,
            }
        } else {
            bail!(
                "unsupported filesystem entry {}; stop source clients before migrating",
                path.display()
            );
        };
        entries.insert(path.strip_prefix(root)?.to_path_buf(), value);
    }
    Ok(entries)
}

fn file_hash(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn staged_file_path(path: &Path, staging: &Path, destination: &Path) -> anyhow::Result<PathBuf> {
    if path.is_symlink() {
        let target = fs::read_link(path)?;
        if let Ok(relative) = target.strip_prefix(destination) {
            return Ok(staging.join(relative));
        }
    }
    Ok(path.to_path_buf())
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path, _source: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path, source: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if source.symlink_metadata()?.file_type().is_symlink_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "credential_tests.rs"]
mod credential_tests;
