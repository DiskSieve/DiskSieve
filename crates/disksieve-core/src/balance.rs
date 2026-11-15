//! Space reconciliation: what the volume reports as used vs. what the scan could account for.
//!
//! The difference ("unscanned / hidden") comes from folders without read permission, shadow
//! copies / snapshots, filesystem metadata, reserved blocks and similar. It is only meaningful
//! when a whole volume was scanned — see [`is_volume_root`].

use std::path::Path;

/// Volume usage compared with the scan result. All values are bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceBalance {
    /// Capacity of the volume.
    pub total: u64,
    /// Used space reported by the OS (`total - available`).
    pub used: u64,
    /// Free space reported by the OS.
    pub free: u64,
    /// Size on disk found by the scan.
    pub scanned: u64,
    /// Used space the scan could not see (`used - scanned`, never negative).
    pub unaccounted: u64,
}

impl SpaceBalance {
    /// Builds the balance from the volume's `total` and `available` space and the scan's
    /// allocated bytes. Inconsistent inputs (available > total, scanned > used — e.g. hard
    /// links counted twice) are clamped instead of underflowing.
    pub fn new(total: u64, available: u64, scanned: u64) -> Self {
        let free = available.min(total);
        let used = total - free;
        Self {
            total,
            used,
            free,
            scanned,
            unaccounted: used.saturating_sub(scanned),
        }
    }

    /// Share of the used space the scan could not see, in `0.0..=1.0`.
    pub fn unaccounted_ratio(&self) -> f64 {
        if self.used == 0 {
            0.0
        } else {
            self.unaccounted as f64 / self.used as f64
        }
    }
}

/// True when `scan_root` is the mount point itself (a whole-volume scan), not a folder on it.
/// Comparison is by path components, so trailing separators do not matter.
pub fn is_volume_root(scan_root: &Path, mount_point: &Path) -> bool {
    scan_root.components().eq(mount_point.components())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn unaccounted_is_used_minus_scanned() {
        let b = SpaceBalance::new(500 * GIB, 200 * GIB, 280 * GIB);
        assert_eq!(b.used, 300 * GIB);
        assert_eq!(b.free, 200 * GIB);
        assert_eq!(b.unaccounted, 20 * GIB);
        assert!((b.unaccounted_ratio() - 20.0 / 300.0).abs() < 1e-9);
    }

    #[test]
    fn scan_larger_than_used_does_not_underflow() {
        let b = SpaceBalance::new(100, 40, 70);
        assert_eq!(b.used, 60);
        assert_eq!(b.unaccounted, 0);
    }

    #[test]
    fn available_above_total_is_clamped() {
        let b = SpaceBalance::new(100, 150, 0);
        assert_eq!((b.used, b.free, b.unaccounted), (0, 100, 0));
        assert_eq!(b.unaccounted_ratio(), 0.0);
    }

    #[test]
    fn volume_root_ignores_trailing_separator() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let with_sep = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        assert!(is_volume_root(Path::new(&with_sep), root));
        assert!(!is_volume_root(&root.join("sub"), root));
        assert!(!is_volume_root(root, &root.join("sub")));
    }
}
