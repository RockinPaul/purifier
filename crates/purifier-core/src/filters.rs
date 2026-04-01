use std::path::{Path, PathBuf};

use glob::Pattern;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileTypeMatch {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardLinkStatus {
    Any,
    IsHardLinked,
    IsNotHardLinked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageStatus {
    Any,
    IsPackage,
    IsNotPackage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterTest {
    NameContains(String),
    PathGlob(String),
    SizeAtLeast(u64),
    SizeAtMost(u64),
    FileType(FileTypeMatch),
    HardLinkStatus(HardLinkStatus),
    PackageStatus(PackageStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    Single(FilterTest),
    All(Vec<Filter>),
    Any(Vec<Filter>),
    Not(Box<Filter>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanProfile {
    pub name: String,
    pub exclude: Option<Filter>,
    pub mask: Option<Filter>,
    pub display_filter: Option<Filter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterEntryMeta {
    pub path: PathBuf,
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub is_dir: bool,
    pub is_package: bool,
    pub hard_link_status: HardLinkStatus,
}

impl Filter {
    pub fn single(test: FilterTest) -> Self {
        Self::Single(test)
    }

    pub fn all<I, T>(filters: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Filter>,
    {
        Self::All(filters.into_iter().map(Into::into).collect())
    }

    pub fn any<I, T>(filters: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Filter>,
    {
        Self::Any(filters.into_iter().map(Into::into).collect())
    }

    pub fn matches(&self, meta: &FilterEntryMeta) -> bool {
        match self {
            Self::Single(test) => test.matches(meta),
            Self::All(filters) => filters.iter().all(|filter| filter.matches(meta)),
            Self::Any(filters) => filters.iter().any(|filter| filter.matches(meta)),
            Self::Not(filter) => !filter.matches(meta),
        }
    }
}

impl From<FilterTest> for Filter {
    fn from(value: FilterTest) -> Self {
        Self::Single(value)
    }
}

impl FilterTest {
    fn matches(&self, meta: &FilterEntryMeta) -> bool {
        match self {
            Self::NameContains(needle) => meta
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(needle)),
            Self::PathGlob(glob) => path_matches_glob(meta.path.as_path(), glob),
            Self::SizeAtLeast(size) => meta.logical_bytes >= *size,
            Self::SizeAtMost(size) => meta.logical_bytes <= *size,
            Self::FileType(file_type) => match file_type {
                FileTypeMatch::File => !meta.is_dir,
                FileTypeMatch::Directory => meta.is_dir,
            },
            Self::HardLinkStatus(status) => match status {
                HardLinkStatus::Any => true,
                HardLinkStatus::IsHardLinked => {
                    matches!(meta.hard_link_status, HardLinkStatus::IsHardLinked)
                }
                HardLinkStatus::IsNotHardLinked => {
                    matches!(meta.hard_link_status, HardLinkStatus::IsNotHardLinked)
                }
            },
            Self::PackageStatus(status) => match status {
                PackageStatus::Any => true,
                PackageStatus::IsPackage => meta.is_package,
                PackageStatus::IsNotPackage => !meta.is_package,
            },
        }
    }
}

impl ScanProfile {
    pub fn should_exclude(&self, path: &Path) -> bool {
        self.exclude
            .as_ref()
            .is_some_and(|filter| filter.matches(&filter_meta_for_path(path)))
    }
}

pub fn built_in_scan_profiles() -> Vec<ScanProfile> {
    vec![
        ScanProfile {
            name: "Full scan".to_string(),
            exclude: None,
            mask: None,
            display_filter: None,
        },
        ScanProfile {
            name: "Fast developer scan".to_string(),
            exclude: Some(Filter::any([
                FilterTest::PathGlob("**/node_modules/**".to_string()),
                FilterTest::PathGlob("**/target/**".to_string()),
                FilterTest::PathGlob("**/DerivedData/**".to_string()),
            ])),
            mask: None,
            display_filter: None,
        },
    ]
}

pub(crate) fn is_package_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "app"
                    | "appex"
                    | "bundle"
                    | "framework"
                    | "kext"
                    | "mdimporter"
                    | "pkg"
                    | "plugin"
                    | "prefpane"
                    | "qlgenerator"
                    | "xpc"
            )
        })
}

fn filter_meta_for_path(path: &Path) -> FilterEntryMeta {
    let is_package = is_package_path(path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            let is_dir = metadata.is_dir();
            let hard_link_status =
                if !metadata.file_type().is_symlink() && !is_dir && metadata.nlink() > 1 {
                    HardLinkStatus::IsHardLinked
                } else {
                    HardLinkStatus::IsNotHardLinked
                };

            return FilterEntryMeta {
                path: path.to_path_buf(),
                logical_bytes: metadata.len(),
                physical_bytes: if is_dir { 0 } else { metadata.blocks() * 512 },
                is_dir,
                is_package,
                hard_link_status,
            };
        }
    }

    #[cfg(not(unix))]
    {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            let is_dir = metadata.is_dir();
            return FilterEntryMeta {
                path: path.to_path_buf(),
                logical_bytes: metadata.len(),
                physical_bytes: if is_dir { 0 } else { metadata.len() },
                is_dir,
                is_package,
                hard_link_status: HardLinkStatus::IsNotHardLinked,
            };
        }
    }

    FilterEntryMeta {
        path: path.to_path_buf(),
        logical_bytes: 0,
        physical_bytes: 0,
        is_dir: path.is_dir(),
        is_package,
        hard_link_status: HardLinkStatus::Any,
    }
}

fn path_matches_glob(path: &Path, glob: &str) -> bool {
    Pattern::new(glob).ok().is_some_and(|pattern| {
        pattern.matches_path(path) || pattern.matches(&format!("{}/", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        built_in_scan_profiles, is_package_path, Filter, FilterEntryMeta, FilterTest,
        HardLinkStatus, PackageStatus, ScanProfile,
    };

    #[test]
    fn filter_should_match_by_path_glob_and_hard_link_status() {
        let filter = Filter::all([
            FilterTest::PathGlob("**/node_modules/**".to_string()),
            FilterTest::HardLinkStatus(HardLinkStatus::Any),
        ]);

        let meta = FilterEntryMeta {
            path: PathBuf::from("/tmp/project/node_modules/pkg/index.js"),
            logical_bytes: 100,
            physical_bytes: 4096,
            is_dir: false,
            is_package: false,
            hard_link_status: HardLinkStatus::Any,
        };

        assert!(filter.matches(&meta));
    }

    #[test]
    fn scan_profile_should_exclude_matching_paths() {
        let profile = ScanProfile {
            name: "exclude-node-modules".to_string(),
            exclude: Some(Filter::single(FilterTest::PathGlob(
                "**/node_modules/**".to_string(),
            ))),
            mask: None,
            display_filter: None,
        };

        assert!(profile.should_exclude(Path::new("/tmp/app/node_modules/react/index.js")));
    }

    #[test]
    fn hard_link_status_should_change_filter_match_result() {
        let filter = Filter::single(FilterTest::HardLinkStatus(HardLinkStatus::IsHardLinked));

        let hard_linked = FilterEntryMeta {
            path: PathBuf::from("/tmp/file"),
            logical_bytes: 100,
            physical_bytes: 4096,
            is_dir: false,
            is_package: false,
            hard_link_status: HardLinkStatus::IsHardLinked,
        };
        let not_hard_linked = FilterEntryMeta {
            hard_link_status: HardLinkStatus::IsNotHardLinked,
            ..hard_linked.clone()
        };

        assert!(filter.matches(&hard_linked));
        assert!(!filter.matches(&not_hard_linked));
    }

    #[test]
    fn package_status_should_match_package_paths() {
        let profile = ScanProfile {
            name: "exclude-app-bundles".to_string(),
            exclude: Some(Filter::single(FilterTest::PackageStatus(
                PackageStatus::IsPackage,
            ))),
            mask: None,
            display_filter: None,
        };

        assert!(profile.should_exclude(Path::new("/Applications/Foo.app")));
        assert!(!profile.should_exclude(Path::new("/tmp/plain-dir")));
    }

    #[test]
    fn package_status_should_match_common_macos_bundles() {
        for path in [
            "/Applications/Foo.app",
            "/System/Library/Frameworks/Bar.framework",
            "/System/Library/PreferencePanes/Baz.prefPane",
            "/System/Library/Extensions/Qux.kext",
            "/System/Library/PrivateFrameworks/Agent.xpc",
            "/Applications/Widget.appex",
        ] {
            assert!(is_package_path(Path::new(path)), "expected package: {path}");
        }

        for path in ["/tmp/plain-dir", "/tmp/readme.txt", "/tmp/plugin"] {
            assert!(
                !is_package_path(Path::new(path)),
                "expected plain path: {path}"
            );
        }
    }

    #[test]
    fn built_in_profile_should_exclude_developer_artifacts() {
        let profile = built_in_scan_profiles()
            .into_iter()
            .find(|profile| profile.name == "Fast developer scan")
            .expect("expected fast developer profile");

        assert!(profile.should_exclude(Path::new("/tmp/app/node_modules/react/index.js")));
        assert!(profile.should_exclude(Path::new("/tmp/app/target/debug/purifier")));
        assert!(profile.should_exclude(Path::new(
            "/Users/test/Library/Developer/Xcode/DerivedData/App/Logs/build.log"
        )));
        assert!(!profile.should_exclude(Path::new("/tmp/app/src/main.rs")));
    }

    #[cfg(unix)]
    #[test]
    fn scan_profile_should_use_real_hard_link_metadata_when_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("file.txt");
        let linked = dir.path().join("file-copy.txt");
        std::fs::write(&original, b"hello").unwrap();
        std::fs::hard_link(&original, &linked).unwrap();

        let profile = ScanProfile {
            name: "exclude-hard-links".to_string(),
            exclude: Some(Filter::single(FilterTest::HardLinkStatus(
                HardLinkStatus::IsHardLinked,
            ))),
            mask: None,
            display_filter: None,
        };

        assert!(profile.should_exclude(&linked));
    }
}
