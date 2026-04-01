use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeMode {
    Physical,
    Logical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntrySizes {
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub accounted_physical_bytes: u64,
}

impl EntrySizes {
    pub fn display_bytes(self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Physical => self.physical_bytes,
            SizeMode::Logical => self.logical_bytes,
        }
    }

    pub fn accounted_total_bytes(self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Physical => self.accounted_physical_bytes,
            SizeMode::Logical => self.logical_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
}

#[cfg(test)]
mod tests {
    use super::{EntrySizes, FileIdentity, SizeMode};

    #[test]
    fn physical_display_bytes_should_use_path_physical_bytes() {
        let sizes = EntrySizes {
            logical_bytes: 100,
            physical_bytes: 4096,
            accounted_physical_bytes: 0,
        };

        assert_eq!(sizes.display_bytes(SizeMode::Physical), 4096);
    }

    #[test]
    fn logical_mode_should_use_logical_bytes() {
        let sizes = EntrySizes {
            logical_bytes: 100,
            physical_bytes: 4096,
            accounted_physical_bytes: 4096,
        };

        assert_eq!(sizes.display_bytes(SizeMode::Logical), 100);
    }

    #[test]
    fn physical_accounted_total_should_use_accounted_physical_bytes() {
        let sizes = EntrySizes {
            logical_bytes: 100,
            physical_bytes: 4096,
            accounted_physical_bytes: 0,
        };

        assert_eq!(sizes.accounted_total_bytes(SizeMode::Physical), 0);
    }

    #[test]
    fn hard_link_identity_should_round_trip() {
        let identity = FileIdentity {
            dev: 7,
            ino: 42,
            nlink: 3,
        };

        assert_eq!(identity.dev, 7);
        assert_eq!(identity.ino, 42);
        assert_eq!(identity.nlink, 3);
    }
}
