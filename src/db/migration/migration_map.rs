// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::bail;
use itertools::Itertools;
use multimap::MultiMap;
use once_cell::sync::Lazy;
use semver::Version;
use tracing::info;

use super::v0_12_1::Migration0_12_1_0_13_0;

/// Migration trait. It is expected that the [`MigrationOperation::migrate`] method will pick up the relevant database
/// existing under `chain_data_path` and create a new migration database in the same directory.
pub(super) trait MigrationOperation {
    /// Performs pre-migration checks. This is the place to check if the database is in a valid
    /// state and if the migration can be performed. Note that some of the higher-level checks
    /// (like checking if the database exists) are performed by the [`Migration`].
    fn pre_checks(&self, chain_data_path: &Path) -> anyhow::Result<()>;
    /// Performs the actual migration. All the logic should be implemented here.
    /// Ideally, the migration should use as little of the Forest codebase as possible to avoid
    /// potential issues with the migration code itself and having to update it in the future.
    /// Returns the path to the migrated database (which is not yet validated)
    fn migrate(&self, chain_data_path: &Path) -> anyhow::Result<PathBuf>;
    /// Performs post-migration checks. This is the place to check if the migration database is
    /// ready to be used by Forest and renamed into a versioned database.
    fn post_checks(&self, chain_data_path: &Path) -> anyhow::Result<()>;
}

/// Migrations map. The key is the starting version and the value is the tuple of the target version
/// and the [`MigrationOperation`] implementation.
///
/// In the future we might want to drop legacy migrations (e.g., to clean-up the database
/// dependency that may get several breaking changes).
// If need be, we should introduce "jump" migrations here, e.g. 0.12.0 -> 0.12.2, 0.12.2 -> 0.12.3, etc.
// This would allow us to skip migrations in case of bugs or just for performance reasons.
type Migrator = Arc<dyn MigrationOperation + Send + Sync>;
type MigrationsMap = MultiMap<Version, (Version, Migrator)>;
pub(super) static MIGRATIONS: Lazy<MigrationsMap> = Lazy::new(|| {
    MigrationsMap::from_iter(
        [(
            Version::new(0, 12, 1),
            (
                Version::new(0, 13, 0),
                Arc::new(Migration0_12_1_0_13_0) as _,
            ),
        )]
        .iter()
        .cloned(),
    )
});

pub struct Migration {
    from: Version,
    to: Version,
    migrator: Migrator,
}

impl Migration {
    pub fn migrate(&self, chain_data_path: &Path) -> anyhow::Result<()> {
        info!(
            "Migrating database from version {} to {}",
            self.from, self.to
        );

        self.pre_checks(chain_data_path)?;
        let migrated_db = self.migrator.migrate(chain_data_path)?;
        self.post_checks(chain_data_path)?;

        let new_db = chain_data_path.join(format!("{}", self.to));
        std::fs::rename(migrated_db, new_db)?;

        let old_db = chain_data_path.join(format!("{}", self.from));
        std::fs::remove_dir_all(old_db)?;

        info!("Database migration complete");
        Ok(())
    }

    fn pre_checks(&self, chain_data_path: &Path) -> anyhow::Result<()> {
        let source_db = chain_data_path.join(self.from.to_string());
        if !source_db.exists() {
            bail!(
                "source database {source_db} does not exist",
                source_db = source_db.display()
            );
        }

        let target_db = chain_data_path.join(self.to.to_string());
        if target_db.exists() {
            bail!(
                "target database {target_db} already exists",
                target_db = target_db.display()
            );
        }

        self.migrator.pre_checks(chain_data_path)
    }

    fn post_checks(&self, chain_data_path: &Path) -> anyhow::Result<()> {
        self.migrator.post_checks(chain_data_path)
    }
}

/// Creates a migration chain from `start` to `goal`. The chain is chosen to be the shortest
/// possible. If there are multiple shortest paths, any of them is chosen. This method will use
/// the pre-defined migrations map.
pub(super) fn create_migration_chain(
    start: &Version,
    goal: &Version,
) -> anyhow::Result<Vec<Migration>> {
    create_migration_chain_from_migrations(start, goal, &MIGRATIONS)
}

/// Same as [`create_migration_chain`], but uses any provided migrations map.
fn create_migration_chain_from_migrations(
    start: &Version,
    goal: &Version,
    migrations_map: &MigrationsMap,
) -> anyhow::Result<Vec<Migration>> {
    let result = pathfinding::directed::bfs::bfs(
        start,
        |from| {
            if let Some(migrations) = migrations_map.get_vec(from) {
                migrations.iter().map(|(to, _)| to.clone()).collect()
            } else {
                vec![]
            }
        },
        |to| to == goal,
    )
    .ok_or_else(|| anyhow::anyhow!("No migration path found from version {start} to {goal}"))?
    .iter()
    .tuple_windows()
    .map(|(from, to)| {
        let migrator = migrations_map
            .get_vec(from)
            .expect("Migration must exist")
            .iter()
            .find(|(version, _)| version == to)
            .expect("Migration must exist")
            .1
            .clone();

        Migration {
            from: from.clone(),
            to: to.clone(),
            migrator,
        }
    })
    .collect_vec();

    if result.is_empty() {
        bail!(
            "No migration path found from version {start} to {goal}",
            start = start,
            goal = goal
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::utils::version::FOREST_VERSION;

    #[test]
    fn test_possible_to_migrate_to_current_version() {
        // This test ensures that it is possible to migrate from the oldest supported version to the current
        // version.
        let earliest_version = MIGRATIONS
            .iter_all()
            .map(|(from, _)| from)
            .min()
            .expect("At least one migration must exist");
        let current_version = &FOREST_VERSION;

        let migrations = create_migration_chain(earliest_version, current_version).unwrap();
        assert!(!migrations.is_empty());
    }

    #[test]
    fn test_ensure_migration_possible_from_anywhere_to_latest() {
        // This test ensures that it is possible to find migration chain from any version to the
        // current version.
        let current_version = &FOREST_VERSION;

        for (from, _) in MIGRATIONS.iter_all() {
            let migrations = create_migration_chain(from, current_version).unwrap();
            assert!(!migrations.is_empty());
        }
    }

    #[test]
    fn test_ensure_migration_not_possible_if_higher_than_latest() {
        // This test ensures that it is not possible to migrate from a version higher than the
        // current version.
        let current_version = &FOREST_VERSION;

        let higher_version = Version::new(
            current_version.major,
            current_version.minor,
            current_version.patch + 1,
        );
        let migrations = create_migration_chain(&higher_version, current_version);
        assert!(migrations.is_err());
    }

    #[test]
    fn test_migration_down_not_possible() {
        // This test ensures that it is not possible to migrate down from the latest version.
        // This is not a strict requirement and we may want to allow this in the future.
        let current_version = &FOREST_VERSION;

        for (from, _) in MIGRATIONS.iter_all() {
            let migrations = create_migration_chain(current_version, from);
            assert!(migrations.is_err());
        }
    }

    #[derive(Debug, Clone)]
    struct EmptyMigration;

    impl MigrationOperation for EmptyMigration {
        fn pre_checks(&self, _chain_data_path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        fn migrate(&self, _chain_data_path: &Path) -> anyhow::Result<PathBuf> {
            Ok("".into())
        }

        fn post_checks(&self, _chain_data_path: &Path) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_migration_should_use_shortest_path() -> anyhow::Result<()> {
        let migrations = MigrationsMap::from_iter(
            [
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 2, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 2, 0),
                    (Version::new(0, 3, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 3, 0), Arc::new(EmptyMigration) as _),
                ),
            ]
            .iter()
            .cloned(),
        );

        let migrations = create_migration_chain_from_migrations(
            &Version::new(0, 1, 0),
            &Version::new(0, 3, 0),
            &migrations,
        )?;

        // The shortest path is 0.1.0 to 0.3.0 (without going through 0.2.0)
        assert_eq!(1, migrations.len());
        assert_eq!(Version::new(0, 1, 0), migrations[0].from);
        assert_eq!(Version::new(0, 3, 0), migrations[0].to);

        Ok(())
    }

    #[test]
    fn test_migration_complex_path() -> anyhow::Result<()> {
        let migrations = MigrationsMap::from_iter(
            [
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 2, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 2, 0),
                    (Version::new(0, 3, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 3, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 3, 0),
                    (Version::new(0, 3, 1), Arc::new(EmptyMigration) as _),
                ),
            ]
            .iter()
            .cloned(),
        );

        let migrations = create_migration_chain_from_migrations(
            &Version::new(0, 1, 0),
            &Version::new(0, 3, 1),
            &migrations,
        )?;

        // The shortest path is 0.1.0 -> 0.3.0 -> 0.3.1
        assert_eq!(2, migrations.len());
        assert_eq!(Version::new(0, 1, 0), migrations[0].from);
        assert_eq!(Version::new(0, 3, 0), migrations[0].to);
        assert_eq!(Version::new(0, 3, 0), migrations[1].from);
        assert_eq!(Version::new(0, 3, 1), migrations[1].to);

        Ok(())
    }

    #[test]
    fn test_same_distance_paths_should_yield_any() -> anyhow::Result<()> {
        let migrations = MigrationsMap::from_iter(
            [
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 2, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 2, 0),
                    (Version::new(0, 4, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 1, 0),
                    (Version::new(0, 3, 0), Arc::new(EmptyMigration) as _),
                ),
                (
                    Version::new(0, 3, 0),
                    (Version::new(0, 4, 0), Arc::new(EmptyMigration) as _),
                ),
            ]
            .iter()
            .cloned(),
        );

        let migrations = create_migration_chain_from_migrations(
            &Version::new(0, 1, 0),
            &Version::new(0, 4, 0),
            &migrations,
        )?;

        // there are two possible shortest paths:
        // 0.1.0 -> 0.2.0 -> 0.4.0
        // 0.1.0 -> 0.3.0 -> 0.4.0
        // Both of them are correct and should be accepted.
        assert_eq!(2, migrations.len());
        if migrations[0].to == Version::new(0, 2, 0) {
            assert_eq!(Version::new(0, 1, 0), migrations[0].from);
            assert_eq!(Version::new(0, 2, 0), migrations[0].to);
            assert_eq!(Version::new(0, 2, 0), migrations[1].from);
            assert_eq!(Version::new(0, 4, 0), migrations[1].to);
        } else {
            assert_eq!(Version::new(0, 1, 0), migrations[0].from);
            assert_eq!(Version::new(0, 3, 0), migrations[0].to);
            assert_eq!(Version::new(0, 3, 0), migrations[1].from);
            assert_eq!(Version::new(0, 4, 0), migrations[1].to);
        }

        Ok(())
    }

    struct SimpleMigration0_1_0_0_2_0;

    impl MigrationOperation for SimpleMigration0_1_0_0_2_0 {
        fn pre_checks(&self, chain_data_path: &Path) -> anyhow::Result<()> {
            let path = chain_data_path.join("0.1.0");
            if !path.exists() {
                anyhow::bail!("0.1.0 does not exist");
            }
            Ok(())
        }

        fn migrate(&self, chain_data_path: &Path) -> anyhow::Result<PathBuf> {
            fs::create_dir(chain_data_path.join("migration_0_1_0_0_2_0"))?;
            Ok(chain_data_path.join("migration_0_1_0_0_2_0"))
        }

        fn post_checks(&self, chain_data_path: &Path) -> anyhow::Result<()> {
            let path = chain_data_path.join("migration_0_1_0_0_2_0");
            if !path.exists() {
                anyhow::bail!("migration_0_1_0_0_2_0 does not exist");
            }
            Ok(())
        }
    }

    #[test]
    fn test_migration_map_migration() -> anyhow::Result<()> {
        let migration = Migration {
            from: Version::new(0, 1, 0),
            to: Version::new(0, 2, 0),
            migrator: Arc::new(SimpleMigration0_1_0_0_2_0),
        };

        let temp_dir = TempDir::new()?;

        assert!(migration.pre_checks(temp_dir.path()).is_err());
        fs::create_dir(temp_dir.path().join("0.1.0"))?;
        assert!(migration.pre_checks(temp_dir.path()).is_ok());

        migration.migrate(temp_dir.path())?;
        assert!(temp_dir.path().join("0.2.0").exists());

        assert!(migration.post_checks(temp_dir.path()).is_err());
        fs::create_dir(temp_dir.path().join("migration_0_1_0_0_2_0"))?;
        assert!(migration.post_checks(temp_dir.path()).is_ok());

        Ok(())
    }
}