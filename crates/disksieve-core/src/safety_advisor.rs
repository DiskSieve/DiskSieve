//! Safety Advisor: "Can I delete this?" for any node (spec 1.1, roadmap step 5.1).
//!
//! A fixed dictionary of deterministic rules matches system files and caches by path
//! (case-insensitive) and gives each a [`SafetyLevel`], an explanation and, where one exists, a
//! safer way to reclaim the space. Dev Junk artifacts get the rebuild hint of their
//! [`JunkKind`]. [`rules_markdown`] renders the dictionary as the user documentation in
//! `docs/safety-advisor.md`.
//!
//! The verdict is tied to the delete [`Guard`]: the level is [`SafetyLevel::System`] exactly
//! when the guard refuses to move the node to Trash, so the UI can never promise something the
//! backend does not enforce. A rule written for a protected location (e.g. `WinSxS`) that
//! matches an unprotected copy elsewhere is reported as [`SafetyLevel::Review`].
//!
//! Pure tree and path logic — no filesystem access.

use std::fmt::Write as _;
use std::path::{Component, Path};

use crate::dev_junk::JunkKind;
use crate::error::{CoreError, CoreResult};
use crate::safety::{Guard, Refusal, TrashError};
use crate::tree::{NodeId, ScanTree, ROOT};

/// How safe it is to delete a node, from the safest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SafetyLevel {
    /// Recreated automatically or rebuildable; moving it to Trash loses nothing important.
    Safe,
    /// May hold something you need; check before deleting.
    Review,
    /// Protected: DiskSieve refuses to move it to Trash (see [`Advice::protected`]).
    System,
}

impl SafetyLevel {
    /// Stable identifier used in IPC: `safe`, `review` or `system`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Review => "review",
            Self::System => "system",
        }
    }
}

/// Operating system a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    /// The platform this binary was built for (other Unix systems use the Linux rules).
    pub const CURRENT: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Linux
    };

    const fn bit(self) -> u8 {
        match self {
            Self::Windows => WIN,
            Self::MacOs => MAC,
            Self::Linux => LINUX,
        }
    }
}

const WIN: u8 = 1;
const MAC: u8 = 2;
const LINUX: u8 = 4;

/// A safer way to reclaim the space than deleting the item by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alternative {
    /// What to do, in words (a settings path, an app action…).
    pub how: &'static str,
    /// Command to run instead, if there is one.
    pub command: Option<&'static str>,
}

/// Which matches of a pattern count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The node is the matched item itself.
    Exact,
    /// The node is the matched folder or anything inside it.
    Subtree,
}

/// Path segments of one location on the platforms in `os`.
///
/// The first segment may be an anchor: `~` (the user's home folder) or `/` (the root of a
/// drive — any drive on Windows). Without an anchor the pattern matches anywhere in the path.
/// Segments compare case-insensitively; `*` matches any one name and a trailing `*` any name
/// starting with the text before it.
#[derive(Debug, Clone, Copy)]
struct Pattern {
    os: u8,
    segs: &'static [&'static str],
}

const HOME: &str = "~";
const VOLUME: &str = "/";

const fn win(segs: &'static [&'static str]) -> Pattern {
    Pattern { os: WIN, segs }
}
const fn mac(segs: &'static [&'static str]) -> Pattern {
    Pattern { os: MAC, segs }
}
const fn linux(segs: &'static [&'static str]) -> Pattern {
    Pattern { os: LINUX, segs }
}
const fn unix(segs: &'static [&'static str]) -> Pattern {
    Pattern {
        os: MAC | LINUX,
        segs,
    }
}
const fn any(segs: &'static [&'static str]) -> Pattern {
    Pattern {
        os: WIN | MAC | LINUX,
        segs,
    }
}

/// One entry of the advisor's dictionary.
#[derive(Debug)]
pub struct Rule {
    /// Stable identifier, e.g. `hiberfil`.
    pub id: &'static str,
    patterns: &'static [Pattern],
    scope: Scope,
    /// For [`Scope::Exact`]: whether the matched item must be a file.
    file_only: bool,
    pub level: SafetyLevel,
    pub title: &'static str,
    pub description: &'static str,
    pub alternative: Option<Alternative>,
}

impl Rule {
    /// Whether any location of the rule exists on `platform`.
    pub fn applies_to(&self, platform: Platform) -> bool {
        self.patterns.iter().any(|p| p.os & platform.bit() != 0)
    }
}

/// Verdict for one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    pub level: SafetyLevel,
    /// Id of the matched rule: a [`Rule::id`], `dev_junk`, or `None` when nothing matched.
    pub rule: Option<&'static str>,
    pub title: &'static str,
    pub description: &'static str,
    pub alternative: Option<Alternative>,
    /// Why the delete guard refuses this node; `Some` exactly when `level` is `System`.
    pub protected: Option<Refusal>,
}

const fn alt(how: &'static str, command: &'static str) -> Option<Alternative> {
    Some(Alternative {
        how,
        command: Some(command),
    })
}

const fn tip(how: &'static str) -> Option<Alternative> {
    Some(Alternative { how, command: None })
}

const CLEANUP: Option<Alternative> = alt(
    "Use Disk Cleanup: Clean up system files, tick the matching category and run it.",
    "cleanmgr.exe",
);

const VIRTUAL_MEMORY: Option<Alternative> = alt(
    "Resize or move it in System Properties: Advanced → Performance → Settings → Advanced → Virtual memory.",
    "SystemPropertiesPerformance.exe",
);

const BROWSER: Option<Alternative> = tip(
    "Close the browser first, or clear it in the browser: Settings → Privacy → Clear browsing data → Cached images and files (Ctrl+Shift+Delete).",
);

const UNINSTALL: Option<Alternative> = alt(
    "Uninstall programs you no longer use: Settings → Apps → Installed apps, or Programs and Features.",
    "appwiz.cpl",
);

const SHADERS: &str = "Precompiled shaders. Games and the graphics driver rebuild them on the next launch; the first start of each game may stutter for a moment.";

const USER_CACHE: &str = "App caches, recreated when needed. Quit the apps first. Some caches (downloaded packages, AI models, browser data) take time or bandwidth to fetch again.";

/// The dictionary, most specific rules first (the first match wins).
pub const RULES: &[Rule] = &[
    // ── Steam and games ───────────────────────────────────────────────────────────────
    Rule {
        id: "steam_shader_cache",
        patterns: &[any(&["steamapps", "shadercache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Steam shader cache",
        description: SHADERS,
        alternative: None,
    },
    Rule {
        id: "steam_downloads",
        patterns: &[
            any(&["steamapps", "downloading"]),
            any(&["steamapps", "temp"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Unfinished Steam downloads",
        description: "Game downloads and updates Steam has not finished yet. Removing them loses the download progress; Steam starts over the next time it updates the game.",
        alternative: tip("Pause or cancel the download in Steam → Downloads first."),
    },
    Rule {
        id: "steam_workshop",
        patterns: &[any(&["steamapps", "workshop"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Steam Workshop content",
        description: "Mods, maps and other community items from the Steam Workshop. Steam downloads the items you are still subscribed to again.",
        alternative: tip("Unsubscribe from the items you no longer use on the game's Workshop page."),
    },
    Rule {
        id: "steam_proton_prefixes",
        patterns: &[linux(&["steamapps", "compatdata"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Proton prefixes",
        description: "Windows environments of games run through Proton. They can hold save games and settings that are not synced to Steam Cloud.",
        alternative: tip("Uninstalling a game in Steam removes its prefix; back up save games first."),
    },
    Rule {
        id: "steam_games",
        patterns: &[any(&["steamapps", "common"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Installed Steam games",
        description: "Games installed by Steam. Deleted by hand, Steam still lists them as installed and offers to repair or download them again. Save games are usually stored elsewhere or in Steam Cloud.",
        alternative: tip("Uninstall in Steam: Library → right-click the game → Manage → Uninstall, or Settings → Storage."),
    },
    // ── Shader caches ─────────────────────────────────────────────────────────────────
    Rule {
        id: "directx_shader_cache",
        patterns: &[win(&["~", "AppData", "Local", "D3DSCache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "DirectX shader cache",
        description: SHADERS,
        alternative: alt("Disk Cleanup → DirectX Shader Cache.", "cleanmgr.exe"),
    },
    Rule {
        id: "nvidia_shader_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "NVIDIA", "DXCache"]),
            win(&["~", "AppData", "Local", "NVIDIA", "GLCache"]),
            win(&["~", "AppData", "Local", "NVIDIA Corporation", "NV_Cache"]),
            linux(&["~", ".cache", "nvidia", "GLCache"]),
            linux(&["~", ".nv", "GLCache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "NVIDIA shader cache",
        description: SHADERS,
        alternative: None,
    },
    Rule {
        id: "amd_shader_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "AMD", "DxCache"]),
            win(&["~", "AppData", "Local", "AMD", "DxcCache"]),
            win(&["~", "AppData", "Local", "AMD", "GLCache"]),
            win(&["~", "AppData", "Local", "AMD", "VkCache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "AMD shader cache",
        description: SHADERS,
        alternative: None,
    },
    Rule {
        id: "intel_shader_cache",
        patterns: &[win(&["~", "AppData", "LocalLow", "Intel", "ShaderCache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Intel shader cache",
        description: SHADERS,
        alternative: None,
    },
    Rule {
        id: "mesa_shader_cache",
        patterns: &[
            linux(&["~", ".cache", "mesa_shader_cache"]),
            linux(&["~", ".cache", "mesa_shader_cache_db"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Mesa shader cache",
        description: SHADERS,
        alternative: None,
    },
    // ── Windows system files and folders ──────────────────────────────────────────────
    Rule {
        id: "hiberfil",
        patterns: &[win(&["hiberfil.sys"])],
        scope: Scope::Exact,
        file_only: true,
        level: SafetyLevel::System,
        title: "Hibernation file",
        description: "Windows saves the memory contents here for hibernation and Fast Startup. Deleted by hand, it comes back at the next restart.",
        alternative: alt(
            "Turn hibernation off (this also disables Fast Startup) in a terminal run as administrator.",
            "powercfg /hibernate off",
        ),
    },
    Rule {
        id: "pagefile",
        patterns: &[win(&["pagefile.sys"])],
        scope: Scope::Exact,
        file_only: true,
        level: SafetyLevel::System,
        title: "Page file",
        description: "Windows virtual memory, in use all the time. Its size is managed by Windows.",
        alternative: VIRTUAL_MEMORY,
    },
    Rule {
        id: "swapfile",
        patterns: &[win(&["swapfile.sys"])],
        scope: Scope::Exact,
        file_only: true,
        level: SafetyLevel::System,
        title: "Swap file",
        description: "Used by Windows to swap Store apps in and out of memory. It is managed together with the page file.",
        alternative: VIRTUAL_MEMORY,
    },
    Rule {
        id: "memory_dump",
        patterns: &[
            win(&["/", "Windows", "MEMORY.DMP"]),
            win(&["/", "Windows", "Minidump"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "System crash dumps",
        description: "Memory dumps Windows wrote after a crash (blue screen). Only useful for finding the cause of the crash.",
        alternative: alt(
            "Disk Cleanup → Clean up system files → System error memory dump files.",
            "cleanmgr.exe",
        ),
    },
    Rule {
        id: "winsxs",
        patterns: &[win(&["/", "Windows", "WinSxS"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows component store",
        description: "Holds the components behind Windows Update and optional features. Many of its files are hard links shared with System32, so it takes less space than shown.",
        alternative: alt(
            "Remove superseded component versions in a terminal run as administrator.",
            "Dism.exe /Online /Cleanup-Image /StartComponentCleanup",
        ),
    },
    Rule {
        id: "windows_update_cache",
        patterns: &[win(&["/", "Windows", "SoftwareDistribution", "Download"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows Update downloads",
        description: "Update packages that Windows Update downloaded. Windows Update may be using them right now.",
        alternative: CLEANUP,
    },
    Rule {
        id: "delivery_optimization",
        patterns: &[
            win(&["/", "Windows", "SoftwareDistribution", "DeliveryOptimization"]),
            win(&[
                "/",
                "Windows",
                "ServiceProfiles",
                "NetworkService",
                "AppData",
                "Local",
                "Microsoft",
                "Windows",
                "DeliveryOptimization",
            ]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Delivery Optimization cache",
        description: "Update files Windows keeps to share with other PCs. Windows trims it by itself.",
        alternative: alt(
            "Disk Cleanup → Delivery Optimization Files, or in PowerShell run as administrator:",
            "Delete-DeliveryOptimizationCache",
        ),
    },
    Rule {
        id: "windows_installer_cache",
        patterns: &[win(&["/", "Windows", "Installer"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows Installer cache",
        description: "Copies of the installers (.msi, .msp) of your programs. Windows needs them to repair, update and uninstall those programs; deleting them breaks all three.",
        alternative: UNINSTALL,
    },
    Rule {
        id: "windows_temp",
        patterns: &[win(&["/", "Windows", "Temp"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows temporary files",
        description: "Temporary files of Windows and installers running as the system.",
        alternative: alt(
            "Settings → System → Storage → Temporary files, or Disk Cleanup.",
            "cleanmgr.exe",
        ),
    },
    Rule {
        id: "windows_prefetch",
        patterns: &[win(&["/", "Windows", "Prefetch"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Prefetch data",
        description: "Small traces Windows uses to start apps faster. Recreated automatically; deleting them only slows down the next starts.",
        alternative: None,
    },
    Rule {
        id: "windows_logs",
        patterns: &[
            win(&["/", "Windows", "Logs"]),
            win(&["/", "Windows", "Panther"]),
            win(&["/", "Windows", "System32", "LogFiles"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows logs",
        description: "Logs of Windows setup, updates and services. Windows rotates most of them by itself.",
        alternative: CLEANUP,
    },
    Rule {
        id: "system32",
        patterns: &[
            win(&["/", "Windows", "System32"]),
            win(&["/", "Windows", "SysWOW64"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows system files",
        description: "Core programs, libraries and drivers of Windows. Deleting anything here can stop Windows from starting.",
        alternative: None,
    },
    Rule {
        id: "windows_dir",
        patterns: &[win(&["/", "Windows"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows folder",
        description: "The operating system itself. Its contents are managed by Windows; deleting files here can break Windows or installed apps.",
        alternative: CLEANUP,
    },
    Rule {
        id: "program_files",
        patterns: &[
            win(&["/", "Program Files"]),
            win(&["/", "Program Files (x86)"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Installed programs",
        description: "Programs installed for all users. Deleting a program's folder leaves it half-installed: shortcuts, registry entries and the uninstaller stay behind.",
        alternative: UNINSTALL,
    },
    Rule {
        id: "package_cache",
        patterns: &[win(&["/", "ProgramData", "Package Cache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Installer package cache",
        description: "Installers kept by Visual Studio, .NET, the Visual C++ runtimes and other programs. They are needed to repair, modify or uninstall those programs.",
        alternative: UNINSTALL,
    },
    Rule {
        id: "windows_error_reports",
        patterns: &[
            win(&["/", "ProgramData", "Microsoft", "Windows", "WER"]),
            win(&["~", "AppData", "Local", "Microsoft", "Windows", "WER"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Windows error reports",
        description: "Reports collected when apps crashed or hung. Only needed to troubleshoot those crashes.",
        alternative: CLEANUP,
    },
    Rule {
        id: "windows_old",
        patterns: &[win(&["/", "Windows.old"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Previous Windows installation",
        description: "Kept after a Windows upgrade so you can go back to the earlier version; Windows removes it by itself after about 10 days. Deleting it by hand usually stops at files owned by the system.",
        alternative: alt(
            "Settings → System → Storage → Temporary files → Previous Windows installation(s), or Disk Cleanup → Clean up system files.",
            "cleanmgr.exe",
        ),
    },
    Rule {
        id: "windows_upgrade_files",
        patterns: &[
            win(&["/", "$Windows.~BT"]),
            win(&["/", "$Windows.~WS"]),
            win(&["/", "$WinREAgent"]),
            win(&["/", "$GetCurrent"]),
            win(&["/", "ESD"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Windows upgrade files",
        description: "Temporary files of a Windows upgrade or feature update. Not needed once the upgrade has finished, but Windows uses them while it is still in progress.",
        alternative: alt(
            "Settings → System → Storage → Temporary files → Temporary Windows installation files, or Disk Cleanup.",
            "cleanmgr.exe",
        ),
    },
    Rule {
        id: "recycle_bin",
        patterns: &[win(&["/", "$Recycle.Bin"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Recycle Bin",
        description: "Files already deleted on this drive, waiting in the Recycle Bin.",
        alternative: alt(
            "Empty the Recycle Bin (right-click it on the desktop), or run this in PowerShell.",
            "Clear-RecycleBin",
        ),
    },
    Rule {
        id: "system_volume_information",
        patterns: &[win(&["/", "System Volume Information"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Restore points and shadow copies",
        description: "System Restore points, shadow copies and indexing data. Only Windows can manage it.",
        alternative: alt(
            "System Protection → Configure: lower the disk space used or delete all restore points.",
            "SystemPropertiesProtection.exe",
        ),
    },
    Rule {
        id: "recovery_dir",
        patterns: &[win(&["/", "Recovery"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Windows recovery files",
        description: "The Windows Recovery Environment, used to repair or reset Windows when it does not start.",
        alternative: None,
    },
    Rule {
        id: "driver_installers",
        patterns: &[
            win(&["/", "AMD"]),
            win(&["/", "Intel"]),
            win(&["/", "NVIDIA"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Unpacked driver installers",
        description: "Files the AMD, Intel or NVIDIA driver setup unpacked before installing. The installed drivers do not use them.",
        alternative: None,
    },
    // ── Windows user folders ──────────────────────────────────────────────────────────
    Rule {
        id: "windows_user_temp",
        patterns: &[win(&["~", "AppData", "Local", "Temp"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Temporary files",
        description: "Temporary files left by apps and installers. Files that are still in use cannot be moved and stay where they are.",
        alternative: tip("Settings → System → Storage → Temporary files."),
    },
    Rule {
        id: "windows_inet_cache",
        patterns: &[win(&["~", "AppData", "Local", "Microsoft", "Windows", "INetCache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Internet cache",
        description: "Web cache of Windows components and older apps (Internet Explorer mode, Office add-ins).",
        alternative: alt("Disk Cleanup → Temporary Internet Files.", "cleanmgr.exe"),
    },
    Rule {
        id: "windows_thumbnail_cache",
        patterns: &[win(&["~", "AppData", "Local", "Microsoft", "Windows", "Explorer"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Thumbnail and icon cache",
        description: "Thumbnails and icons File Explorer keeps to show folders faster. Rebuilt as you browse; files in use stay.",
        alternative: alt("Disk Cleanup → Thumbnails.", "cleanmgr.exe"),
    },
    Rule {
        id: "crash_dumps",
        patterns: &[win(&["~", "AppData", "Local", "CrashDumps"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "App crash dumps",
        description: "Memory dumps of apps that crashed. Only useful for finding the cause of those crashes.",
        alternative: None,
    },
    // ── Browsers ──────────────────────────────────────────────────────────────────────
    Rule {
        id: "browser_cache",
        patterns: &[
            // Chromium browsers on Windows: <vendor>\<browser>\User Data\<profile>\...
            win(&["~", "AppData", "Local", "Google", "Chrome", "User Data", "*", "Cache"]),
            win(&["~", "AppData", "Local", "Google", "Chrome", "User Data", "*", "Code Cache"]),
            win(&["~", "AppData", "Local", "Google", "Chrome", "User Data", "*", "GPUCache"]),
            win(&["~", "AppData", "Local", "Microsoft", "Edge", "User Data", "*", "Cache"]),
            win(&["~", "AppData", "Local", "Microsoft", "Edge", "User Data", "*", "Code Cache"]),
            win(&["~", "AppData", "Local", "Microsoft", "Edge", "User Data", "*", "GPUCache"]),
            win(&["~", "AppData", "Local", "BraveSoftware", "Brave-Browser", "User Data", "*", "Cache"]),
            win(&["~", "AppData", "Local", "BraveSoftware", "Brave-Browser", "User Data", "*", "Code Cache"]),
            win(&["~", "AppData", "Local", "Vivaldi", "User Data", "*", "Cache"]),
            win(&["~", "AppData", "Local", "Opera Software", "*", "Cache"]),
            win(&["~", "AppData", "Local", "Mozilla", "Firefox", "Profiles", "*", "cache2"]),
            linux(&["~", ".cache", "google-chrome"]),
            linux(&["~", ".cache", "chromium"]),
            linux(&["~", ".cache", "microsoft-edge"]),
            linux(&["~", ".cache", "BraveSoftware"]),
            linux(&["~", ".cache", "vivaldi"]),
            linux(&["~", ".cache", "mozilla", "firefox"]),
            mac(&["~", "Library", "Caches", "Google", "Chrome"]),
            mac(&["~", "Library", "Caches", "Microsoft Edge"]),
            mac(&["~", "Library", "Caches", "BraveSoftware"]),
            mac(&["~", "Library", "Caches", "Firefox"]),
            mac(&["~", "Library", "Caches", "com.apple.Safari"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Browser cache",
        description: "Copies of web pages, images and scripts. The browser downloads them again when needed; your history, passwords and bookmarks are kept elsewhere.",
        alternative: BROWSER,
    },
    // ── Cloud and personal folders ────────────────────────────────────────────────────
    Rule {
        id: "cloud_sync",
        patterns: &[
            win(&["~", "OneDrive*"]),
            win(&["~", "iCloudDrive"]),
            any(&["~", "Dropbox"]),
            mac(&["~", "Library", "CloudStorage"]),
            mac(&["~", "Library", "Mobile Documents"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Cloud-synced folder",
        description: "Synced with a cloud service: deleting a file here also deletes it in the cloud and on your other devices.",
        alternative: tip("To free space but keep the files in the cloud, right-click them and choose Free up space (OneDrive), Make online-only (Dropbox) or Remove Download (iCloud)."),
    },
    Rule {
        id: "downloads",
        patterns: &[any(&["~", "Downloads"])],
        scope: Scope::Exact,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Downloads folder",
        description: "Old installers and archives pile up here, but it can also hold the only copy of a file. Sort by date or size and go through it.",
        alternative: None,
    },
    // ── Apps ──────────────────────────────────────────────────────────────────────────
    Rule {
        id: "electron_app_cache",
        patterns: &[
            win(&["~", "AppData", "Roaming", "*", "Cache"]),
            win(&["~", "AppData", "Roaming", "*", "Code Cache"]),
            win(&["~", "AppData", "Roaming", "*", "GPUCache"]),
            win(&["~", "AppData", "Roaming", "*", "CachedData"]),
            mac(&["~", "Library", "Application Support", "*", "Cache"]),
            mac(&["~", "Library", "Application Support", "*", "Code Cache"]),
            mac(&["~", "Library", "Application Support", "*", "GPUCache"]),
            mac(&["~", "Library", "Application Support", "*", "CachedData"]),
            linux(&["~", ".config", "*", "Cache"]),
            linux(&["~", ".config", "*", "Code Cache"]),
            linux(&["~", ".config", "*", "GPUCache"]),
            linux(&["~", ".config", "*", "CachedData"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Desktop app cache",
        description: "Cache of a desktop app built on Chromium or Electron (VS Code, Discord, Slack, Teams…). Recreated when the app starts; quit the app first.",
        alternative: None,
    },
    Rule {
        id: "spotify_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "Spotify", "Storage"]),
            win(&["~", "AppData", "Local", "Packages", "SpotifyAB.SpotifyMusic_*", "LocalCache"]),
            mac(&["~", "Library", "Caches", "com.spotify.client"]),
            linux(&["~", ".cache", "spotify"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Spotify cache",
        description: "Music Spotify streamed recently. Songs downloaded for offline listening are kept here too and would have to be downloaded again.",
        alternative: tip("In Spotify: Settings → Storage → Clear cache."),
    },
    Rule {
        id: "store_app_temp",
        patterns: &[win(&["~", "AppData", "Local", "Packages", "*", "TempState"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Store app temporary files",
        description: "Temporary files of an app installed from the Microsoft Store. Windows may delete them at any time.",
        alternative: None,
    },
    Rule {
        id: "docker_data",
        patterns: &[
            win(&["~", "AppData", "Local", "Docker"]),
            mac(&["~", "Library", "Containers", "com.docker.docker"]),
            linux(&["/", "var", "lib", "docker"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Docker data",
        description: "Docker images, containers, volumes and build cache, on Windows and macOS stored in one large disk image. Deleting it by hand loses every container and volume.",
        alternative: alt(
            "Remove unused images, stopped containers and the build cache (add --volumes to include unused volumes). On Windows and macOS the disk image may not shrink until Docker Desktop compacts it.",
            "docker system prune -a",
        ),
    },
    Rule {
        id: "wsl_disk",
        patterns: &[win(&["ext4.vhdx"])],
        scope: Scope::Exact,
        file_only: true,
        level: SafetyLevel::Review,
        title: "WSL virtual disk",
        description: "The disk of a WSL Linux distribution, with the whole system and your files in it. Deleting it deletes the distribution.",
        alternative: alt(
            "Unregister distributions you no longer need (this deletes their files); list them with wsl -l -v.",
            "wsl --unregister <distro>",
        ),
    },
    Rule {
        id: "store_app_data",
        patterns: &[win(&["~", "AppData", "Local", "Packages"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Store app data",
        description: "Settings, saved games and offline content of apps installed from the Microsoft Store.",
        alternative: tip("Settings → Apps → Installed apps → the app → Advanced options → Reset, or uninstall the app."),
    },
    Rule {
        id: "virtual_machines",
        patterns: &[
            any(&["~", "VirtualBox VMs"]),
            win(&["~", "Documents", "Virtual Machines"]),
            win(&["/", "ProgramData", "Microsoft", "Windows", "Virtual Hard Disks"]),
            win(&["/", "Users", "Public", "Documents", "Hyper-V", "Virtual Hard Disks"]),
            mac(&["~", "Virtual Machines.localized"]),
            mac(&["~", "Parallels"]),
            mac(&["~", "Library", "Containers", "com.utmapp.UTM"]),
            linux(&["~", "vmware"]),
            linux(&["/", "var", "lib", "libvirt", "images"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Virtual machines",
        description: "Virtual machines and their disks. Deleting a disk deletes everything installed in that machine.",
        alternative: tip("Delete machines in the virtualization app (VirtualBox, VMware, Hyper-V, Parallels, UTM, virt-manager) so it also forgets them."),
    },
    // ── Developer caches and tools ────────────────────────────────────────────────────
    Rule {
        id: "npm_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "npm-cache"]),
            unix(&["~", ".npm"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "npm cache",
        description: "Packages npm downloaded. They are downloaded again by the next install that needs them.",
        alternative: alt("Clear it with npm.", "npm cache clean --force"),
    },
    Rule {
        id: "pnpm_store",
        patterns: &[
            win(&["~", "AppData", "Local", "pnpm", "store"]),
            mac(&["~", "Library", "pnpm", "store"]),
            linux(&["~", ".local", "share", "pnpm", "store"]),
            any(&["~", ".pnpm-store"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "pnpm store",
        description: "Packages shared by all your pnpm projects through hard links, so they take space only once. Projects keep working when it is deleted; their next install downloads the packages again.",
        alternative: alt("Remove only the packages no project uses.", "pnpm store prune"),
    },
    Rule {
        id: "yarn_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "Yarn", "Cache"]),
            mac(&["~", "Library", "Caches", "Yarn"]),
            linux(&["~", ".cache", "yarn"]),
            any(&["~", ".yarn", "berry", "cache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Yarn cache",
        description: "Packages Yarn downloaded. They are downloaded again by the next install that needs them.",
        alternative: alt("Clear it with Yarn.", "yarn cache clean"),
    },
    Rule {
        id: "bun_cache",
        patterns: &[any(&["~", ".bun", "install", "cache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Bun cache",
        description: "Packages Bun downloaded. They are downloaded again by the next install that needs them.",
        alternative: alt("Clear it with Bun.", "bun pm cache rm"),
    },
    Rule {
        id: "cargo_registry",
        patterns: &[
            any(&["~", ".cargo", "registry"]),
            any(&["~", ".cargo", "git"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Cargo registry cache",
        description: "Crates and git dependencies Cargo downloaded and unpacked. Cargo downloads them again on the next build.",
        alternative: alt(
            "Trim old entries with cargo-cache (install it with cargo install cargo-cache).",
            "cargo cache --autoclean",
        ),
    },
    Rule {
        id: "rustup_toolchains",
        patterns: &[any(&["~", ".rustup", "toolchains"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Rust toolchains",
        description: "Rust compilers installed by rustup. Deleting folders by hand confuses rustup.",
        alternative: alt(
            "Uninstall the toolchains you no longer use; list them with rustup toolchain list.",
            "rustup toolchain uninstall <toolchain>",
        ),
    },
    Rule {
        id: "go_module_cache",
        patterns: &[any(&["~", "go", "pkg", "mod"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Go module cache",
        description: "Modules Go downloaded. They are downloaded again on the next build. The files are read-only, so deleting them by hand often fails.",
        alternative: alt("Clear it with Go.", "go clean -modcache"),
    },
    Rule {
        id: "go_build_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "go-build"]),
            mac(&["~", "Library", "Caches", "go-build"]),
            linux(&["~", ".cache", "go-build"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Go build cache",
        description: "Compiled packages Go reuses between builds. The next build recreates them.",
        alternative: alt("Clear it with Go.", "go clean -cache"),
    },
    Rule {
        id: "gradle_cache",
        patterns: &[
            any(&["~", ".gradle", "caches"]),
            any(&["~", ".gradle", "wrapper", "dists"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Gradle cache",
        description: "Dependencies, build cache and Gradle versions downloaded by Gradle. Downloaded again on the next build.",
        alternative: alt("Stop the Gradle daemons before deleting it.", "gradle --stop"),
    },
    Rule {
        id: "maven_repository",
        patterns: &[any(&["~", ".m2", "repository"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Maven local repository",
        description: "Libraries Maven downloaded, downloaded again on the next build. Artifacts you installed yourself with mvn install have to be built again.",
        alternative: None,
    },
    Rule {
        id: "nuget_cache",
        patterns: &[
            any(&["~", ".nuget", "packages"]),
            win(&["~", "AppData", "Local", "NuGet", "v3-cache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "NuGet cache",
        description: "Packages NuGet downloaded for .NET projects. Restored again on the next build.",
        alternative: alt("Clear all NuGet caches.", "dotnet nuget locals all --clear"),
    },
    Rule {
        id: "pip_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "pip", "cache"]),
            mac(&["~", "Library", "Caches", "pip"]),
            linux(&["~", ".cache", "pip"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "pip cache",
        description: "Python packages pip downloaded or built. Installed packages are not affected.",
        alternative: alt("Clear it with pip.", "pip cache purge"),
    },
    Rule {
        id: "uv_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "uv", "cache"]),
            unix(&["~", ".cache", "uv"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "uv cache",
        description: "Python packages uv downloaded. Environments created by uv link to it and need a reinstall after it is deleted.",
        alternative: alt("Clear it with uv.", "uv cache clean"),
    },
    Rule {
        id: "conda_packages",
        patterns: &[
            any(&["~", "anaconda3", "pkgs"]),
            any(&["~", "miniconda3", "pkgs"]),
            any(&["~", "miniforge3", "pkgs"]),
            any(&["~", ".conda", "pkgs"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Conda package cache",
        description: "Package archives Conda downloaded. Existing environments keep working.",
        alternative: alt("Clean it with Conda.", "conda clean --all"),
    },
    Rule {
        id: "composer_cache",
        patterns: &[
            win(&["~", "AppData", "Local", "Composer"]),
            mac(&["~", "Library", "Caches", "composer"]),
            linux(&["~", ".cache", "composer"]),
            unix(&["~", ".composer", "cache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Composer cache",
        description: "PHP packages Composer downloaded. Downloaded again on the next install.",
        alternative: alt("Clear it with Composer.", "composer clear-cache"),
    },
    Rule {
        id: "pub_cache",
        patterns: &[
            any(&["~", ".pub-cache"]),
            win(&["~", "AppData", "Local", "Pub", "Cache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Dart / Flutter package cache",
        description: "Packages downloaded by dart pub and flutter pub. Downloaded again on the next pub get.",
        alternative: alt("Clear it with Dart.", "dart pub cache clean"),
    },
    Rule {
        id: "cocoapods_cache",
        patterns: &[mac(&["~", "Library", "Caches", "CocoaPods"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "CocoaPods cache",
        description: "Pods CocoaPods downloaded. Downloaded again on the next pod install.",
        alternative: alt("Clear it with CocoaPods.", "pod cache clean --all"),
    },
    Rule {
        id: "homebrew_cache",
        patterns: &[
            mac(&["~", "Library", "Caches", "Homebrew"]),
            linux(&["~", ".cache", "Homebrew"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Homebrew cache",
        description: "Downloaded bottles and source archives of Homebrew packages. Installed packages are not affected.",
        alternative: alt(
            "Remove the cache and old versions of installed packages.",
            "brew cleanup --prune=all",
        ),
    },
    Rule {
        id: "huggingface_models",
        patterns: &[any(&["~", ".cache", "huggingface"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Hugging Face models",
        description: "AI models and datasets downloaded by Hugging Face libraries. They are downloaded again when needed, which can take a long time.",
        alternative: alt("Pick the models to delete interactively.", "huggingface-cli delete-cache"),
    },
    Rule {
        id: "ollama_models",
        patterns: &[any(&["~", ".ollama", "models"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Ollama models",
        description: "Local AI models pulled by Ollama. Their files are shared between models, so delete them with Ollama rather than by hand.",
        alternative: alt(
            "Remove models you no longer use; list them with ollama list.",
            "ollama rm <model>",
        ),
    },
    Rule {
        id: "lm_studio_models",
        patterns: &[any(&["~", ".lmstudio", "models"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "LM Studio models",
        description: "Local AI models downloaded by LM Studio. They are downloaded again when needed, which can take a long time.",
        alternative: tip("Delete models in LM Studio → My Models."),
    },
    Rule {
        id: "jetbrains_toolbox",
        patterns: &[
            win(&["~", "AppData", "Local", "JetBrains", "Toolbox"]),
            mac(&["~", "Library", "Application Support", "JetBrains", "Toolbox"]),
            linux(&["~", ".local", "share", "JetBrains", "Toolbox"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "JetBrains Toolbox IDEs",
        description: "IDEs installed by JetBrains Toolbox, possibly in several versions.",
        alternative: tip("Uninstall old IDE versions in Toolbox (the ⋮ menu next to each IDE)."),
    },
    Rule {
        id: "jetbrains_caches",
        patterns: &[
            win(&["~", "AppData", "Local", "JetBrains"]),
            mac(&["~", "Library", "Caches", "JetBrains"]),
            linux(&["~", ".cache", "JetBrains"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "JetBrains IDE caches",
        description: "Indexes and caches of JetBrains IDEs, often for versions no longer installed. Rebuilt when a project is opened, which takes a while; close the IDE first.",
        alternative: tip("In the IDE: File → Invalidate Caches. Folders of old IDE versions can go as a whole."),
    },
    Rule {
        id: "android_sdk",
        patterns: &[
            win(&["~", "AppData", "Local", "Android", "Sdk"]),
            mac(&["~", "Library", "Android", "sdk"]),
            linux(&["~", "Android", "Sdk"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Android SDK",
        description: "Android platforms, build tools, system images and the emulator. Projects that target a deleted platform stop building until it is installed again.",
        alternative: tip("Remove the parts you don't need in Android Studio → Settings → Android SDK."),
    },
    Rule {
        id: "android_emulators",
        patterns: &[any(&["~", ".android", "avd"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Android emulators",
        description: "Android virtual devices with their disks, apps and snapshots.",
        alternative: alt(
            "Delete devices in Android Studio → Device Manager, or with avdmanager.",
            "avdmanager delete avd -n <name>",
        ),
    },
    Rule {
        id: "android_cache",
        patterns: &[
            any(&["~", ".android", "cache"]),
            any(&["~", ".android", "build-cache"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Android build cache",
        description: "Caches of the Android build tools. Recreated by the next build.",
        alternative: None,
    },
    Rule {
        id: "vscode_extensions",
        patterns: &[
            any(&["~", ".vscode", "extensions"]),
            any(&["~", ".vscode-insiders", "extensions"]),
            any(&["~", ".cursor", "extensions"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Editor extensions",
        description: "Extensions installed in VS Code or an editor based on it. Old versions are cleaned up by the editor itself.",
        alternative: tip("Uninstall extensions you no longer use in the editor's Extensions view."),
    },
    // ── Xcode (macOS) ─────────────────────────────────────────────────────────────────
    Rule {
        id: "xcode_derived_data",
        patterns: &[
            mac(&["~", "Library", "Developer", "Xcode", "DerivedData"]),
            mac(&["~", "Library", "Developer", "Xcode", "UserData", "Previews"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Xcode DerivedData",
        description: "Build products, indexes and SwiftUI previews of Xcode projects. Xcode rebuilds them on the next build; quit Xcode first.",
        alternative: None,
    },
    Rule {
        id: "xcode_device_support",
        patterns: &[
            mac(&["~", "Library", "Developer", "Xcode", "iOS DeviceSupport"]),
            mac(&["~", "Library", "Developer", "Xcode", "watchOS DeviceSupport"]),
            mac(&["~", "Library", "Developer", "Xcode", "tvOS DeviceSupport"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Xcode device support files",
        description: "Debug symbols for the OS versions of devices you connected. Xcode copies them again the next time a device with that version is connected.",
        alternative: None,
    },
    Rule {
        id: "xcode_archives",
        patterns: &[mac(&["~", "Library", "Developer", "Xcode", "Archives"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Xcode archives",
        description: "App builds you archived for distribution. They are needed to symbolicate crash reports of released versions; keep the ones you still support.",
        alternative: tip("Delete old archives in Xcode: Window → Organizer → Archives."),
    },
    Rule {
        id: "ios_simulators",
        patterns: &[mac(&["~", "Library", "Developer", "CoreSimulator", "Devices"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "iOS simulators",
        description: "Simulator devices with the apps and data installed on them.",
        alternative: alt(
            "Remove simulators whose runtime is no longer installed.",
            "xcrun simctl delete unavailable",
        ),
    },
    Rule {
        id: "coresimulator_caches",
        patterns: &[mac(&["~", "Library", "Developer", "CoreSimulator", "Caches"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Simulator caches",
        description: "Caches of the iOS simulators, recreated when a simulator starts.",
        alternative: None,
    },
    // ── Apple devices and macOS ───────────────────────────────────────────────────────
    Rule {
        id: "ios_backups",
        patterns: &[
            mac(&["~", "Library", "Application Support", "MobileSync", "Backup"]),
            win(&["~", "AppData", "Roaming", "Apple Computer", "MobileSync", "Backup"]),
            win(&["~", "Apple", "MobileSync", "Backup"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "iPhone and iPad backups",
        description: "Device backups made by Finder, iTunes or the Apple Devices app. It may be the only backup of a device.",
        alternative: tip("Delete old backups in Finder (select the device → Manage Backups) or in iTunes / Apple Devices settings."),
    },
    Rule {
        id: "ios_updates",
        patterns: &[
            mac(&["~", "Library", "iTunes", "iPhone Software Updates"]),
            mac(&["~", "Library", "iTunes", "iPad Software Updates"]),
            win(&["~", "AppData", "Roaming", "Apple Computer", "iTunes", "iPhone Software Updates"]),
            win(&["~", "AppData", "Roaming", "Apple Computer", "iTunes", "iPad Software Updates"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "iOS software updates",
        description: "iOS and iPadOS installers downloaded to update or restore a device. Downloaded again when needed.",
        alternative: None,
    },
    Rule {
        id: "macos_logs",
        patterns: &[mac(&["~", "Library", "Logs"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "App logs",
        description: "Log files written by apps. Only useful when troubleshooting.",
        alternative: None,
    },
    Rule {
        id: "macos_mail",
        patterns: &[
            mac(&["~", "Library", "Mail"]),
            mac(&["~", "Library", "Containers", "com.apple.mail"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Apple Mail data",
        description: "Messages and attachments stored by Apple Mail. Deleting files here damages the mailboxes.",
        alternative: tip("Delete large messages in Mail, or let Mail keep fewer attachments: Mail → Settings → Accounts → Download Attachments."),
    },
    Rule {
        id: "macos_messages",
        patterns: &[mac(&["~", "Library", "Messages"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Messages history",
        description: "Your Messages conversations and their attachments.",
        alternative: tip("Messages → Settings → General → Keep messages, or System Settings → General → Storage → Messages."),
    },
    Rule {
        id: "macos_temp",
        patterns: &[mac(&["/", "private", "var", "folders"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "System caches and temporary files",
        description: "Per-user caches and temporary files managed by macOS. Cleaned up by the system, mostly at restart.",
        alternative: tip("Restart the Mac."),
    },
    Rule {
        id: "macos_swap",
        patterns: &[mac(&["/", "private", "var", "vm"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "Swap and sleep image",
        description: "macOS virtual memory and the sleep image, managed by the system. Swap shrinks after a restart.",
        alternative: None,
    },
    Rule {
        id: "macos_system",
        patterns: &[mac(&["/", "System"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::System,
        title: "macOS system",
        description: "The operating system, on a sealed read-only volume.",
        alternative: None,
    },
    // ── Linux system ──────────────────────────────────────────────────────────────────
    Rule {
        id: "apt_cache",
        patterns: &[linux(&["/", "var", "cache", "apt"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "APT package cache",
        description: "Packages APT downloaded to install or update software. Installed software is not affected.",
        alternative: alt("Clear it with APT.", "sudo apt clean"),
    },
    Rule {
        id: "dnf_cache",
        patterns: &[
            linux(&["/", "var", "cache", "dnf"]),
            linux(&["/", "var", "cache", "yum"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "DNF package cache",
        description: "Packages and metadata DNF downloaded. Installed software is not affected.",
        alternative: alt("Clear it with DNF.", "sudo dnf clean all"),
    },
    Rule {
        id: "pacman_cache",
        patterns: &[linux(&["/", "var", "cache", "pacman", "pkg"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Pacman package cache",
        description: "Every package version pacman downloaded, kept to allow downgrades.",
        alternative: alt(
            "Keep only the latest version of each package (paccache comes with pacman-contrib).",
            "sudo paccache -rk1",
        ),
    },
    Rule {
        id: "systemd_journal",
        patterns: &[linux(&["/", "var", "log", "journal"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "System journal",
        description: "System logs collected by systemd-journald. Needed to troubleshoot past problems.",
        alternative: alt("Shrink the journal to a size of your choice.", "sudo journalctl --vacuum-size=200M"),
    },
    Rule {
        id: "snap_packages",
        patterns: &[linux(&["/", "var", "lib", "snapd", "snaps"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Snap packages",
        description: "Installed snaps, including older disabled revisions kept for rollback.",
        alternative: alt(
            "Keep fewer old revisions (2 is the minimum); snap list --all shows the disabled ones.",
            "sudo snap set system refresh.retain=2",
        ),
    },
    Rule {
        id: "flatpak",
        patterns: &[
            linux(&["/", "var", "lib", "flatpak"]),
            linux(&["~", ".local", "share", "flatpak"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Flatpak apps and runtimes",
        description: "Apps installed with Flatpak and the runtimes they share.",
        alternative: alt("Remove runtimes no app uses any more.", "flatpak uninstall --unused"),
    },
    Rule {
        id: "linux_swapfile",
        patterns: &[linux(&["/", "swapfile"]), linux(&["/", "swap.img"])],
        scope: Scope::Exact,
        file_only: true,
        level: SafetyLevel::Review,
        title: "Swap file",
        description: "Linux swap space, in use while swap is on.",
        alternative: alt(
            "Turn swap off and remove its line from /etc/fstab before deleting it.",
            "sudo swapoff /swapfile",
        ),
    },
    Rule {
        id: "linux_coredumps",
        patterns: &[linux(&["/", "var", "lib", "systemd", "coredump"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "Crash dumps",
        description: "Memory dumps of crashed programs collected by systemd-coredump. Only useful for finding the cause of those crashes.",
        alternative: None,
    },
    // ── Generic user caches and trash (after every more specific rule) ────────────────
    Rule {
        id: "macos_user_caches",
        patterns: &[mac(&["~", "Library", "Caches"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "App caches",
        description: USER_CACHE,
        alternative: None,
    },
    Rule {
        id: "linux_user_cache",
        patterns: &[linux(&["~", ".cache"])],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Safe,
        title: "App caches",
        description: USER_CACHE,
        alternative: None,
    },
    Rule {
        id: "user_trash",
        patterns: &[
            linux(&["~", ".local", "share", "Trash"]),
            mac(&["~", ".Trash"]),
        ],
        scope: Scope::Subtree,
        file_only: false,
        level: SafetyLevel::Review,
        title: "Trash",
        description: "Files you already deleted. Emptying the Trash removes them for good.",
        alternative: tip("Empty the Trash in the file manager (Finder on macOS)."),
    },
];

/// Rule id reported for Dev Junk artifacts.
pub const DEV_JUNK_RULE: &str = "dev_junk";

/// Matches nodes against [`RULES`] for one platform and home folder.
#[derive(Debug, Clone)]
pub struct Advisor {
    platform: Platform,
    /// Lower-case components of the home folder (see [`path_components`]); empty if unknown.
    home: Vec<String>,
}

impl Advisor {
    pub fn new(platform: Platform, home: Option<&Path>) -> Self {
        Self {
            platform,
            home: home.map(path_components).unwrap_or_default(),
        }
    }

    /// The first rule of this platform matching the lower-case path components `comps`.
    pub fn rule_for(&self, comps: &[String], is_dir: bool) -> Option<&'static Rule> {
        let bit = self.platform.bit();
        RULES.iter().find(|r| {
            !(r.scope == Scope::Exact && r.file_only && is_dir)
                && r.patterns
                    .iter()
                    .any(|p| p.os & bit != 0 && self.matches(p.segs, r.scope, comps))
        })
    }

    /// Whether `segs` matches `comps` (the whole item for [`Scope::Exact`], the item or one of
    /// its ancestors for [`Scope::Subtree`]).
    fn matches(&self, segs: &[&str], scope: Scope, comps: &[String]) -> bool {
        let at = |start: usize, segs: &[&str]| {
            let end = start + segs.len();
            comps
                .get(start..end)
                .is_some_and(|window| segs.iter().zip(window).all(|(s, c)| segment_matches(s, c)))
                && (scope == Scope::Subtree || end == comps.len())
        };
        match segs.split_first() {
            Some((&HOME, rest)) => {
                !self.home.is_empty() && comps.starts_with(&self.home) && at(self.home.len(), rest)
            }
            Some((&VOLUME, rest)) => at(self.volume_start(comps), rest),
            _ => (0..=comps.len().saturating_sub(segs.len())).any(|start| at(start, segs)),
        }
    }

    /// Index of the first name below the drive root (skips a Windows drive or UNC prefix).
    fn volume_start(&self, comps: &[String]) -> usize {
        let prefixed = self.platform == Platform::Windows
            && comps
                .first()
                .is_some_and(|c| c.ends_with(':') || c.starts_with(r"\\"));
        usize::from(prefixed)
    }

    /// Verdict for node `id`: the matching rule (or Dev Junk hint), combined with `guard`.
    pub fn advise(&self, tree: &ScanTree, id: NodeId, guard: &Guard) -> CoreResult<Advice> {
        let node = tree.node(id)?;
        let comps = node_components(tree, id);
        let base = if let Some(rule) = self.rule_for(&comps, node.is_dir()) {
            Advice {
                level: rule.level,
                rule: Some(rule.id),
                title: rule.title,
                description: rule.description,
                alternative: rule.alternative,
                protected: None,
            }
        } else if let Some(kind) = JunkKind::of(tree, id) {
            // Unreal `Saved` also holds autosaves and config, unlike the other folders.
            let review =
                kind == JunkKind::UnrealCache && tree.name(id).eq_ignore_ascii_case("saved");
            Advice {
                level: if review {
                    SafetyLevel::Review
                } else {
                    SafetyLevel::Safe
                },
                rule: Some(DEV_JUNK_RULE),
                title: "Rebuildable build artifact",
                description: kind.rebuild_hint(),
                alternative: None,
                protected: None,
            }
        } else {
            Advice {
                level: SafetyLevel::Review,
                rule: None,
                title: "No rule for this item",
                description: "DiskSieve has no specific advice for it. Check what it is before deleting it; Move to Trash can be undone.",
                alternative: None,
                protected: None,
            }
        };
        let protected = match guard.check(tree, id) {
            Ok(_) => None,
            Err(TrashError::Refused(r)) => Some(r),
            Err(TrashError::InvalidNode(id)) => return Err(CoreError::InvalidNode(id)),
            // `Guard::check` does not touch the disk, so nothing else can come back.
            Err(_) => None,
        };
        Ok(combine(base, protected))
    }
}

/// Case-insensitive segment match: `*` is any name, a trailing `*` any name with that prefix.
fn segment_matches(seg: &str, comp: &str) -> bool {
    match seg.strip_suffix('*') {
        Some("") => true,
        Some(prefix) => comp
            .get(..prefix.len())
            .is_some_and(|c| c.eq_ignore_ascii_case(prefix)),
        None => seg.eq_ignore_ascii_case(comp),
    }
}

/// `System` exactly when the guard refuses; a system rule outside a protected place is only
/// worth a review.
fn combine(mut advice: Advice, protected: Option<Refusal>) -> Advice {
    match protected {
        Some(refusal) => {
            if advice.rule.is_none() {
                advice.title = "Protected by DiskSieve";
                advice.description = "DiskSieve never moves this item to Trash.";
            }
            advice.level = SafetyLevel::System;
            advice.protected = Some(refusal);
        }
        None if advice.level == SafetyLevel::System => advice.level = SafetyLevel::Review,
        None => {}
    }
    advice
}

/// Lower-case components of a path: the drive prefix (e.g. `c:`) and every name; the root
/// separator is dropped.
pub fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Prefix(p) => Some(p.as_os_str().to_string_lossy().to_lowercase()),
            Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            Component::RootDir | Component::CurDir | Component::ParentDir => None,
        })
        .collect()
}

/// Components of node `id`: the scan root path, then the names below it.
fn node_components(tree: &ScanTree, id: NodeId) -> Vec<String> {
    let mut comps = path_components(tree.root_path());
    let start = comps.len();
    if id != ROOT {
        comps.push(tree.name(id).to_lowercase());
        for a in tree.ancestors(id).filter(|&a| a != ROOT) {
            comps.push(tree.name(a).to_lowercase());
        }
        comps[start..].reverse();
    }
    comps
}

// ── User documentation ────────────────────────────────────────────────────────────────

const DOCS_INTRO: &str = "<!-- Generated from crates/disksieve-core/src/safety_advisor.rs — do not edit by hand.
     Regenerate: DISKSIEVE_BLESS=1 cargo test -p disksieve-core docs_are_up_to_date -->

# Safety Advisor rules

When you select an item, the Inspector answers **“Can I delete this?”** with one of three verdicts, an explanation and — where there is one — a safer way to get the space back than deleting files by hand.

| Verdict | Meaning |
| :--- | :--- |
| ✅ **Safe** | Recreated automatically or rebuildable. Moving it to Trash loses nothing important. |
| ⚠️ **Review** | May hold something you need. Check before deleting. |
| 🛡️ **Protected** | DiskSieve refuses to move it to Trash. Use the safer way shown instead. |

## How the rules work

- The verdict is decided offline from the item's path — DiskSieve never opens or uploads your files.
- Rules are checked from top to bottom; the first one that matches wins. Matching ignores letter case.
- A rule covers the folder and everything inside it, unless it says *this item only*.
- In the locations below, `C:\\` and `/` mean the root of a drive (on Windows: any drive), `%USERPROFILE%` and `~` mean your home folder, and `…` means anywhere on the disk. `*` stands for any one folder name, and `OneDrive*` for any name starting with “OneDrive”.
- Build artifacts of your projects (`node_modules`, `target`, `.venv`, `bin/obj` and others recognized by their project files — see Dev Junk in the app) are **Safe** and come with a hint on how to rebuild them; Unreal Engine `Saved` folders are **Review**, as they also hold autosaves.
- Anything without a rule is **Review**.

Some places are **Protected** no matter what a rule says, because DiskSieve never moves them to Trash: drive roots, your home folder and the folders containing it, operating system folders (`C:\\Windows`, `C:\\Program Files`, `/System`, `/usr`, …), DiskSieve's own files, online-only cloud files, links, junctions and mount points. A rule for a protected location that matches a copy somewhere else (say, a `WinSxS` folder in a backup) gives **Review** instead.

Everything DiskSieve deletes goes to the system Trash, so it can be restored.
";

const DOCS_OUTRO: &str = "
## Suggest a rule

Missing a folder that eats your disk, or a verdict that looks wrong? Open an issue or a pull request: rules live in [`crates/disksieve-core/src/safety_advisor.rs`](../crates/disksieve-core/src/safety_advisor.rs), each with a test example, and this page is generated from them.
";

/// The rules as the Markdown page `docs/safety-advisor.md`, one table per platform.
pub fn rules_markdown() -> String {
    let mut md = String::from(DOCS_INTRO);
    for (platform, name) in [
        (Platform::Windows, "Windows"),
        (Platform::MacOs, "macOS"),
        (Platform::Linux, "Linux"),
    ] {
        let rules: Vec<&Rule> = RULES.iter().filter(|r| r.applies_to(platform)).collect();
        let _ = write!(
            md,
            "\n## {name}\n\n{} rules.\n\n| Location | Verdict | What it is | Safer way |\n| :--- | :--- | :--- | :--- |\n",
            rules.len()
        );
        for rule in rules {
            let mut location = rule
                .patterns
                .iter()
                .filter(|p| p.os & platform.bit() != 0)
                .map(|p| format!("`{}`", display_pattern(platform, p.segs)))
                .collect::<Vec<_>>()
                .join("<br>");
            if rule.scope == Scope::Exact {
                location.push_str("<br>*this item only*");
            }
            let verdict = match rule.level {
                SafetyLevel::Safe => "✅ Safe",
                SafetyLevel::Review => "⚠️ Review",
                SafetyLevel::System => "🛡️ Protected",
            };
            let safer = rule.alternative.map_or_else(
                || "—".to_owned(),
                |a| match a.command {
                    Some(cmd) => format!("{}<br>`{}`", cell(a.how), cell(cmd)),
                    None => cell(a.how),
                },
            );
            let _ = writeln!(
                md,
                "| {location} | {verdict} | **{}** — {} | {safer} |",
                cell(rule.title),
                cell(rule.description)
            );
        }
    }
    md.push_str(DOCS_OUTRO);
    md
}

/// A pattern written the way the platform shows paths.
fn display_pattern(platform: Platform, segs: &[&str]) -> String {
    let windows = platform == Platform::Windows;
    let sep = if windows { "\\" } else { "/" };
    let (lead, rest) = match segs.split_first() {
        Some((&HOME, rest)) => (if windows { "%USERPROFILE%\\" } else { "~/" }, rest),
        Some((&VOLUME, rest)) => (if windows { "C:\\" } else { "/" }, rest),
        _ => (if windows { "…\\" } else { "…/" }, segs),
    };
    format!("{lead}{}", rest.join(sep))
}

/// Text safe inside a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EntryMeta, NodeFlags, TreeBuilder};
    use std::collections::HashSet;
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::time::Duration;

    const WIN_HOME: &str = r"c:/users/ada";
    const MAC_HOME: &str = "users/ada";
    const LINUX_HOME: &str = "home/ada";

    fn comps(path: &str) -> Vec<String> {
        path.split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    fn home_of(platform: Platform) -> &'static str {
        match platform {
            Platform::Windows => WIN_HOME,
            Platform::MacOs => MAC_HOME,
            Platform::Linux => LINUX_HOME,
        }
    }

    fn advisor(platform: Platform) -> Advisor {
        Advisor {
            platform,
            home: comps(home_of(platform)),
        }
    }

    /// `path` with a leading `~` replaced by the platform's home.
    fn expand(platform: Platform, path: &str) -> String {
        match path.strip_prefix('~') {
            Some(rest) => format!("{}{rest}", home_of(platform)),
            None => path.to_owned(),
        }
    }

    /// Rule id for `path` (a leading `~` = home of the platform).
    fn rule(platform: Platform, path: &str, is_dir: bool) -> Option<&'static str> {
        advisor(platform)
            .rule_for(&comps(&expand(platform, path)), is_dir)
            .map(|r| r.id)
    }

    use Platform::{Linux, MacOs, Windows};

    /// Positive examples, at least one per rule (paths are case-insensitive, `~` = home).
    const EXAMPLES: &[(Platform, &str, bool, &str)] = &[
        // Steam and games
        (Windows, "D:/SteamLibrary/steamapps/shadercache", true, "steam_shader_cache"),
        (Linux, "~/.steam/steam/steamapps/shadercache/570", true, "steam_shader_cache"),
        (MacOs, "~/Library/Application Support/Steam/steamapps/shadercache", true, "steam_shader_cache"),
        (Windows, "D:/SteamLibrary/steamapps/downloading/570", true, "steam_downloads"),
        (Linux, "~/.local/share/Steam/steamapps/temp", true, "steam_downloads"),
        (Windows, "D:/SteamLibrary/steamapps/workshop/content", true, "steam_workshop"),
        (Linux, "~/.local/share/Steam/steamapps/compatdata/1091500", true, "steam_proton_prefixes"),
        (Windows, "D:/SteamLibrary/steamapps/common/Baldurs Gate 3", true, "steam_games"),
        (Linux, "~/.local/share/Steam/steamapps/common/Hades", true, "steam_games"),
        // Shader caches
        (Windows, "~/AppData/Local/D3DSCache", true, "directx_shader_cache"),
        (Windows, "~/AppData/Local/NVIDIA/DXCache", true, "nvidia_shader_cache"),
        (Windows, "~/AppData/Local/NVIDIA/GLCache/abc", true, "nvidia_shader_cache"),
        (Windows, "~/AppData/Local/NVIDIA Corporation/NV_Cache", true, "nvidia_shader_cache"),
        (Linux, "~/.nv/GLCache", true, "nvidia_shader_cache"),
        (Linux, "~/.cache/nvidia/GLCache", true, "nvidia_shader_cache"),
        (Windows, "~/AppData/Local/AMD/DxCache", true, "amd_shader_cache"),
        (Windows, "~/AppData/Local/AMD/VkCache", true, "amd_shader_cache"),
        (Windows, "~/AppData/LocalLow/Intel/ShaderCache", true, "intel_shader_cache"),
        (Linux, "~/.cache/mesa_shader_cache", true, "mesa_shader_cache"),
        // Windows system
        (Windows, "C:/hiberfil.sys", false, "hiberfil"),
        (Windows, "C:/pagefile.sys", false, "pagefile"),
        (Windows, "D:/pagefile.sys", false, "pagefile"),
        (Windows, "C:/swapfile.sys", false, "swapfile"),
        (Windows, "C:/Windows/MEMORY.DMP", false, "memory_dump"),
        (Windows, "C:/Windows/Minidump/112226-1.dmp", false, "memory_dump"),
        (Windows, "C:/Windows/WinSxS", true, "winsxs"),
        (Windows, "C:/Windows/WinSxS/amd64_foo/bar.dll", false, "winsxs"),
        (Windows, "C:/Windows/SoftwareDistribution/Download", true, "windows_update_cache"),
        (Windows, "C:/Windows/SoftwareDistribution/DeliveryOptimization", true, "delivery_optimization"),
        (Windows, "C:/Windows/ServiceProfiles/NetworkService/AppData/Local/Microsoft/Windows/DeliveryOptimization/Cache", true, "delivery_optimization"),
        (Windows, "C:/Windows/Installer", true, "windows_installer_cache"),
        (Windows, "C:/Windows/Installer/1a2b3c.msi", false, "windows_installer_cache"),
        (Windows, "C:/Windows/Temp", true, "windows_temp"),
        (Windows, "C:/Windows/Prefetch", true, "windows_prefetch"),
        (Windows, "C:/Windows/Logs/CBS", true, "windows_logs"),
        (Windows, "C:/Windows/System32/LogFiles", true, "windows_logs"),
        (Windows, "C:/Windows/System32/drivers", true, "system32"),
        (Windows, "C:/Windows/SysWOW64", true, "system32"),
        (Windows, "C:/Windows", true, "windows_dir"),
        (Windows, "C:/Windows/Fonts", true, "windows_dir"),
        (Windows, "C:/Program Files/Adobe", true, "program_files"),
        (Windows, "C:/Program Files (x86)", true, "program_files"),
        (Windows, "C:/ProgramData/Package Cache", true, "package_cache"),
        (Windows, "C:/ProgramData/Microsoft/Windows/WER/ReportArchive", true, "windows_error_reports"),
        (Windows, "~/AppData/Local/Microsoft/Windows/WER", true, "windows_error_reports"),
        (Windows, "C:/Windows.old", true, "windows_old"),
        (Windows, "C:/Windows.old/Users", true, "windows_old"),
        (Windows, "C:/$Windows.~BT", true, "windows_upgrade_files"),
        (Windows, "C:/$WinREAgent", true, "windows_upgrade_files"),
        (Windows, "C:/ESD", true, "windows_upgrade_files"),
        (Windows, "D:/$Recycle.Bin", true, "recycle_bin"),
        (Windows, "C:/System Volume Information", true, "system_volume_information"),
        (Windows, "C:/Recovery", true, "recovery_dir"),
        (Windows, "C:/AMD/AMD-Software-Installer", true, "driver_installers"),
        (Windows, "C:/NVIDIA/DisplayDriver", true, "driver_installers"),
        // Windows user folders
        (Windows, "~/AppData/Local/Temp", true, "windows_user_temp"),
        (Windows, "~/AppData/Local/Temp/setup.log", false, "windows_user_temp"),
        (Windows, "~/AppData/Local/Microsoft/Windows/INetCache", true, "windows_inet_cache"),
        (Windows, "~/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db", false, "windows_thumbnail_cache"),
        (Windows, "~/AppData/Local/CrashDumps", true, "crash_dumps"),
        // Browsers
        (Windows, "~/AppData/Local/Google/Chrome/User Data/Default/Cache", true, "browser_cache"),
        (Windows, "~/AppData/Local/Google/Chrome/User Data/Profile 2/Code Cache", true, "browser_cache"),
        (Windows, "~/AppData/Local/Microsoft/Edge/User Data/Default/Cache/Cache_Data", true, "browser_cache"),
        (Windows, "~/AppData/Local/Mozilla/Firefox/Profiles/x1y2.default-release/cache2", true, "browser_cache"),
        (Linux, "~/.cache/google-chrome/Default/Cache", true, "browser_cache"),
        (Linux, "~/.cache/mozilla/firefox/x1y2.default/cache2", true, "browser_cache"),
        (MacOs, "~/Library/Caches/Google/Chrome/Default/Cache", true, "browser_cache"),
        (MacOs, "~/Library/Caches/com.apple.Safari", true, "browser_cache"),
        // Cloud and personal folders
        (Windows, "~/OneDrive", true, "cloud_sync"),
        (Windows, "~/OneDrive - Contoso/Documents", true, "cloud_sync"),
        (Linux, "~/Dropbox/Photos", true, "cloud_sync"),
        (MacOs, "~/Library/Mobile Documents/com~apple~CloudDocs", true, "cloud_sync"),
        (MacOs, "~/Library/CloudStorage/GoogleDrive-ada", true, "cloud_sync"),
        (Windows, "~/Downloads", true, "downloads"),
        (Linux, "~/Downloads", true, "downloads"),
        // Apps
        (Windows, "~/AppData/Roaming/Code/CachedData", true, "electron_app_cache"),
        (Windows, "~/AppData/Roaming/discord/Cache", true, "electron_app_cache"),
        (MacOs, "~/Library/Application Support/Slack/Cache", true, "electron_app_cache"),
        (Linux, "~/.config/Code/GPUCache", true, "electron_app_cache"),
        (Windows, "~/AppData/Local/Spotify/Storage", true, "spotify_cache"),
        (Windows, "~/AppData/Local/Packages/SpotifyAB.SpotifyMusic_zpdnekdrzrea0/LocalCache", true, "spotify_cache"),
        (MacOs, "~/Library/Caches/com.spotify.client", true, "spotify_cache"),
        (Linux, "~/.cache/spotify", true, "spotify_cache"),
        (Windows, "~/AppData/Local/Packages/Microsoft.Photos_8wekyb3d8bbwe/TempState", true, "store_app_temp"),
        (Windows, "~/AppData/Local/Docker/wsl/disk/docker_data.vhdx", false, "docker_data"),
        (MacOs, "~/Library/Containers/com.docker.docker", true, "docker_data"),
        (Linux, "/var/lib/docker", true, "docker_data"),
        (Windows, "~/AppData/Local/Packages/CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc/LocalState/ext4.vhdx", false, "wsl_disk"),
        (Windows, "D:/WSL/Debian/ext4.vhdx", false, "wsl_disk"),
        (Windows, "~/AppData/Local/Packages/Microsoft.MinecraftUWP_8wekyb3d8bbwe", true, "store_app_data"),
        (Windows, "~/VirtualBox VMs/Ubuntu", true, "virtual_machines"),
        (Windows, "C:/ProgramData/Microsoft/Windows/Virtual Hard Disks", true, "virtual_machines"),
        (MacOs, "~/Parallels/Windows 11.pvm", true, "virtual_machines"),
        (Linux, "/var/lib/libvirt/images", true, "virtual_machines"),
        // Developer caches and tools
        (Windows, "~/AppData/Local/npm-cache", true, "npm_cache"),
        (Linux, "~/.npm/_cacache", true, "npm_cache"),
        (Windows, "~/AppData/Local/pnpm/store/v3", true, "pnpm_store"),
        (Linux, "~/.local/share/pnpm/store", true, "pnpm_store"),
        (MacOs, "~/Library/pnpm/store", true, "pnpm_store"),
        (Windows, "~/AppData/Local/Yarn/Cache", true, "yarn_cache"),
        (Linux, "~/.cache/yarn", true, "yarn_cache"),
        (MacOs, "~/Library/Caches/Yarn", true, "yarn_cache"),
        (Linux, "~/.bun/install/cache", true, "bun_cache"),
        (Windows, "~/.cargo/registry", true, "cargo_registry"),
        (Linux, "~/.cargo/git/checkouts", true, "cargo_registry"),
        (Windows, "~/.rustup/toolchains/nightly-x86_64-pc-windows-msvc", true, "rustup_toolchains"),
        (Linux, "~/go/pkg/mod", true, "go_module_cache"),
        (Windows, "~/AppData/Local/go-build", true, "go_build_cache"),
        (Linux, "~/.cache/go-build", true, "go_build_cache"),
        (MacOs, "~/Library/Caches/go-build", true, "go_build_cache"),
        (Windows, "~/.gradle/caches", true, "gradle_cache"),
        (Linux, "~/.gradle/wrapper/dists", true, "gradle_cache"),
        (MacOs, "~/.m2/repository", true, "maven_repository"),
        (Windows, "~/.nuget/packages", true, "nuget_cache"),
        (Windows, "~/AppData/Local/NuGet/v3-cache", true, "nuget_cache"),
        (Windows, "~/AppData/Local/pip/cache", true, "pip_cache"),
        (Linux, "~/.cache/pip", true, "pip_cache"),
        (MacOs, "~/Library/Caches/pip", true, "pip_cache"),
        (Windows, "~/AppData/Local/uv/cache", true, "uv_cache"),
        (MacOs, "~/.cache/uv", true, "uv_cache"),
        (Linux, "~/miniconda3/pkgs", true, "conda_packages"),
        (Windows, "~/anaconda3/pkgs", true, "conda_packages"),
        (Windows, "~/AppData/Local/Composer", true, "composer_cache"),
        (Linux, "~/.cache/composer", true, "composer_cache"),
        (Windows, "~/AppData/Local/Pub/Cache", true, "pub_cache"),
        (MacOs, "~/.pub-cache", true, "pub_cache"),
        (MacOs, "~/Library/Caches/CocoaPods", true, "cocoapods_cache"),
        (MacOs, "~/Library/Caches/Homebrew/downloads", true, "homebrew_cache"),
        (Linux, "~/.cache/Homebrew", true, "homebrew_cache"),
        (Windows, "~/.cache/huggingface/hub", true, "huggingface_models"),
        (Linux, "~/.cache/huggingface", true, "huggingface_models"),
        (Windows, "~/.ollama/models/blobs", true, "ollama_models"),
        (MacOs, "~/.lmstudio/models", true, "lm_studio_models"),
        (Windows, "~/AppData/Local/JetBrains/Toolbox/apps", true, "jetbrains_toolbox"),
        (MacOs, "~/Library/Application Support/JetBrains/Toolbox", true, "jetbrains_toolbox"),
        (Windows, "~/AppData/Local/JetBrains/IntelliJIdea2025.3", true, "jetbrains_caches"),
        (Linux, "~/.cache/JetBrains", true, "jetbrains_caches"),
        (Windows, "~/AppData/Local/Android/Sdk/system-images", true, "android_sdk"),
        (Linux, "~/Android/Sdk", true, "android_sdk"),
        (Windows, "~/.android/avd/Pixel_8.avd", true, "android_emulators"),
        (Linux, "~/.android/cache", true, "android_cache"),
        (Windows, "~/.vscode/extensions", true, "vscode_extensions"),
        (MacOs, "~/.cursor/extensions", true, "vscode_extensions"),
        // Xcode
        (MacOs, "~/Library/Developer/Xcode/DerivedData", true, "xcode_derived_data"),
        (MacOs, "~/Library/Developer/Xcode/DerivedData/App-abc/Build", true, "xcode_derived_data"),
        (MacOs, "~/Library/Developer/Xcode/iOS DeviceSupport/17.4 (21E219)", true, "xcode_device_support"),
        (MacOs, "~/Library/Developer/Xcode/Archives", true, "xcode_archives"),
        (MacOs, "~/Library/Developer/CoreSimulator/Devices", true, "ios_simulators"),
        (MacOs, "~/Library/Developer/CoreSimulator/Caches", true, "coresimulator_caches"),
        // Apple devices and macOS
        (MacOs, "~/Library/Application Support/MobileSync/Backup", true, "ios_backups"),
        (Windows, "~/AppData/Roaming/Apple Computer/MobileSync/Backup", true, "ios_backups"),
        (Windows, "~/Apple/MobileSync/Backup", true, "ios_backups"),
        (MacOs, "~/Library/iTunes/iPhone Software Updates", true, "ios_updates"),
        (MacOs, "~/Library/Logs/DiagnosticReports", true, "macos_logs"),
        (MacOs, "~/Library/Mail/V10", true, "macos_mail"),
        (MacOs, "~/Library/Messages/Attachments", true, "macos_messages"),
        (MacOs, "/private/var/folders/xy/abc", true, "macos_temp"),
        (MacOs, "/private/var/vm/sleepimage", false, "macos_swap"),
        (MacOs, "/System/Library", true, "macos_system"),
        // Linux system
        (Linux, "/var/cache/apt/archives", true, "apt_cache"),
        (Linux, "/var/cache/dnf", true, "dnf_cache"),
        (Linux, "/var/cache/pacman/pkg", true, "pacman_cache"),
        (Linux, "/var/log/journal", true, "systemd_journal"),
        (Linux, "/var/lib/snapd/snaps", true, "snap_packages"),
        (Linux, "/var/lib/flatpak", true, "flatpak"),
        (Linux, "~/.local/share/flatpak", true, "flatpak"),
        (Linux, "/swapfile", false, "linux_swapfile"),
        (Linux, "/swap.img", false, "linux_swapfile"),
        (Linux, "/var/lib/systemd/coredump", true, "linux_coredumps"),
        // Generic user caches and trash
        (MacOs, "~/Library/Caches", true, "macos_user_caches"),
        (MacOs, "~/Library/Caches/com.example.app", true, "macos_user_caches"),
        (Linux, "~/.cache", true, "linux_user_cache"),
        (Linux, "~/.cache/thumbnails", true, "linux_user_cache"),
        (Linux, "~/.local/share/Trash", true, "user_trash"),
        (MacOs, "~/.Trash", true, "user_trash"),
    ];

    #[test]
    fn every_rule_matches_its_examples() {
        for &(platform, path, is_dir, expected) in EXAMPLES {
            assert_eq!(
                rule(platform, path, is_dir),
                Some(expected),
                "{platform:?} {path}"
            );
        }
    }

    #[test]
    fn every_rule_has_an_example() {
        let tested: HashSet<_> = EXAMPLES.iter().map(|e| e.3).collect();
        for r in RULES {
            assert!(tested.contains(r.id), "rule {} has no example", r.id);
        }
    }

    #[test]
    fn rule_ids_are_unique_and_patterns_well_formed() {
        let mut ids = HashSet::new();
        for r in RULES {
            assert!(ids.insert(r.id), "duplicate rule id {}", r.id);
            assert!(!r.patterns.is_empty(), "{}", r.id);
            assert!(!r.file_only || r.scope == Scope::Exact, "{}", r.id);
            for p in r.patterns {
                assert!(p.os != 0, "{}", r.id);
                for (i, s) in p.segs.iter().enumerate() {
                    let anchor = *s == HOME || *s == VOLUME;
                    assert!(!anchor || i == 0, "{}: anchor inside {:?}", r.id, p.segs);
                    assert!(!s.is_empty() && s.trim() == *s, "{}: {s:?}", r.id);
                    assert!(anchor || !s.contains(['/', '\\']), "{}: {s}", r.id);
                    assert!(!s.trim_end_matches('*').contains('*'), "{}: {s}", r.id);
                }
                let names = p.segs.len() - usize::from(p.segs[0] == HOME || p.segs[0] == VOLUME);
                assert!(names > 0, "{}: {:?}", r.id, p.segs);
            }
        }
    }

    #[test]
    fn system_verdicts_are_enforced_by_the_guard() {
        // `System` rules must name places the guard refuses, and the others places it allows,
        // or `combine` would silently change the verdict shown in the docs.
        let guard = Guard::new(None, []);
        for &(platform, path, _, id) in EXAMPLES {
            if (platform == Windows) != cfg!(windows) {
                continue;
            }
            let full = expand(platform, path);
            let path_buf = if cfg!(windows) {
                PathBuf::from(full)
            } else {
                PathBuf::from(format!("/{}", full.trim_start_matches('/')))
            };
            let r = RULES.iter().find(|r| r.id == id).unwrap();
            assert_eq!(
                guard.check_path(&path_buf).is_err(),
                r.level == SafetyLevel::System,
                "{path} ({id})"
            );
        }
    }

    #[test]
    fn rules_apply_only_on_their_platform() {
        assert_eq!(rule(Linux, "/hiberfil.sys", false), None);
        assert_eq!(rule(MacOs, "~/AppData/Local/Temp", true), None);
        assert_eq!(rule(Windows, "~/Library/Caches", true), None);
        assert_eq!(rule(Windows, "~/.cache", true), None);
        assert_eq!(rule(Linux, "/System/Library", true), None);
        assert_eq!(rule(MacOs, "/var/cache/apt", true), None);
        assert_eq!(rule(Windows, "D:/x/steamapps/compatdata", true), None);
        assert_eq!(
            rule(Linux, "~/Library/Developer/Xcode/DerivedData", true),
            None
        );
    }

    #[test]
    fn near_misses_do_not_match() {
        // Exact rules: a folder with the name, or a file inside it, is something else.
        assert_eq!(rule(Windows, "C:/hiberfil.sys", true), None);
        assert_eq!(rule(Windows, "C:/backup/pagefile.sys.bak", false), None);
        assert_eq!(rule(Windows, "D:/WSL/ext4.vhdx", true), None);
        assert_eq!(rule(Windows, "~/Downloads/setup.exe", false), None);
        // Volume-anchored rules only match right below a drive root.
        assert_eq!(rule(Windows, "D:/backup/Windows/WinSxS", true), None);
        assert_eq!(rule(Windows, "~/Recovery", true), None);
        assert_eq!(rule(Linux, "~/var/cache/apt", true), None);
        assert_eq!(rule(Windows, "D:/Windows/WinSxS", true), Some("winsxs"));
        // Parents of a rule's folder are not covered by it.
        assert_eq!(rule(Windows, "~/AppData/Local/NVIDIA", true), None);
        assert_eq!(
            rule(
                Windows,
                "~/AppData/Local/Google/Chrome/User Data/Default",
                true
            ),
            None
        );
        assert_eq!(rule(MacOs, "~/Library", true), None);
        assert_eq!(rule(Windows, "~/AppData/Roaming/Code", true), None);
        // Home-anchored rules need the home folder: another user's cache is not "yours".
        assert_eq!(rule(Linux, "/home/bob/.cache", true), None);
        assert_eq!(rule(Linux, "~/projects/app/.cache", true), None);
        assert_eq!(rule(Windows, "D:/AppData/Local/Temp", true), None);
        // `Temp` and caches elsewhere are not the Windows user temp folder.
        assert_eq!(rule(Windows, "~/Documents/Temp", true), None);
        // A prefix glob needs the prefix.
        assert_eq!(rule(Windows, "~/MyOneDrive", true), None);
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(rule(Windows, "C:/HIBERFIL.SYS", false), Some("hiberfil"));
        assert_eq!(rule(Windows, "c:/windows/winsxs", true), Some("winsxs"));
        assert_eq!(
            rule(Windows, "~/onedrive - CONTOSO", true),
            Some("cloud_sync")
        );
        assert_eq!(rule(Linux, "~/.CACHE/x", true), Some("linux_user_cache"));
    }

    #[test]
    fn specific_rules_win_over_generic_ones() {
        let cases = [
            (Linux, "~/.cache/google-chrome", "browser_cache"),
            (MacOs, "~/Library/Caches/Firefox/Profiles", "browser_cache"),
            (Linux, "~/.cache/nvidia/GLCache", "nvidia_shader_cache"),
            (Linux, "~/.cache/pip/http", "pip_cache"),
            (Windows, "C:/Windows/System32/LogFiles/WMI", "windows_logs"),
            (Windows, "C:/Windows/WinSxS/Temp", "winsxs"),
            (
                Windows,
                "C:/Program Files (x86)/Steam/steamapps/common/x",
                "steam_games",
            ),
            (
                Windows,
                "~/AppData/Local/Docker/wsl/data/ext4.vhdx",
                "docker_data",
            ),
            (
                Windows,
                "~/AppData/Local/JetBrains/Toolbox",
                "jetbrains_toolbox",
            ),
            (
                Windows,
                "~/AppData/Local/Packages/SpotifyAB.SpotifyMusic_x/LocalCache/a",
                "spotify_cache",
            ),
        ];
        for (platform, path, expected) in cases {
            assert_eq!(rule(platform, path, true), Some(expected), "{path}");
        }
    }

    #[test]
    fn unknown_home_disables_home_rules() {
        let a = Advisor::new(Linux, None);
        assert!(a.rule_for(&comps("home/ada/.cache"), true).is_none());
        assert_eq!(
            a.rule_for(&comps("d/steamapps/shadercache"), true)
                .map(|r| r.id),
            Some("steam_shader_cache")
        );
    }

    #[test]
    fn path_components_keep_the_drive_and_drop_the_root() {
        let base = std::env::temp_dir();
        let c = path_components(&base.join("Some Dir").join("File.TXT"));
        assert_eq!(&c[c.len() - 2..], ["some dir", "file.txt"]);
        assert!(!c.iter().any(|s| s.is_empty() || s == "/" || s == "\\"));
        #[cfg(windows)]
        assert!(c[0].ends_with(':'), "{c:?}");
    }

    // ── User documentation ───────────────────────────────────────────────────────────

    #[test]
    fn patterns_display_like_the_platform_paths() {
        assert_eq!(
            display_pattern(Windows, &["~", "AppData", "Local", "Temp"]),
            r"%USERPROFILE%\AppData\Local\Temp"
        );
        assert_eq!(display_pattern(Windows, &["/", "Windows"]), r"C:\Windows");
        assert_eq!(
            display_pattern(Linux, &["steamapps", "common"]),
            "…/steamapps/common"
        );
        assert_eq!(display_pattern(MacOs, &["/", "System"]), "/System");
        assert_eq!(cell("a | b"), r"a \| b");
    }

    #[test]
    fn docs_list_every_rule_once_per_platform() {
        let md = rules_markdown();
        for r in RULES {
            assert!(md.contains(&format!("**{}**", cell(r.title))), "{}", r.id);
        }
        let rows = md.lines().filter(|l| l.starts_with("| `")).count();
        let expected: usize = [Windows, MacOs, Linux]
            .iter()
            .map(|&p| RULES.iter().filter(|r| r.applies_to(p)).count())
            .sum();
        assert_eq!(rows, expected);
    }

    /// `docs/safety-advisor.md` must match the rules. Regenerate it with
    /// `DISKSIEVE_BLESS=1 cargo test -p disksieve-core docs_are_up_to_date`.
    #[test]
    fn docs_are_up_to_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root")
            .join("docs")
            .join("safety-advisor.md");
        let expected = rules_markdown();
        if std::env::var_os("DISKSIEVE_BLESS").is_some() {
            std::fs::write(&path, &expected).expect("write docs");
            return;
        }
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            actual.replace("\r\n", "\n") == expected,
            "{} is out of date — run: DISKSIEVE_BLESS=1 cargo test -p disksieve-core docs_are_up_to_date",
            path.display()
        );
    }

    // ── Verdicts on a scan tree, combined with the delete guard ──────────────────────

    fn dir() -> EntryMeta {
        EntryMeta {
            logical_size: 0,
            allocated_size: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        }
    }

    fn file() -> EntryMeta {
        EntryMeta {
            logical_size: 10,
            allocated_size: 4096,
            mtime: 0,
            flags: NodeFlags::empty(),
        }
    }

    /// Tree rooted at a home folder (paths only — nothing is read from disk):
    /// `.cache/pip`, `notes.txt`, `game/Saved`, `game/Intermediate`, `game/x.uproject`.
    fn home_tree(home: &Path) -> (ScanTree, [NodeId; 5]) {
        let mut b = TreeBuilder::new(home, 1, dir());
        let cache = b.push(ROOT, OsStr::new(".cache"), dir()).unwrap();
        let pip = b.push(cache, OsStr::new("pip"), dir()).unwrap();
        let notes = b.push(ROOT, OsStr::new("notes.txt"), file()).unwrap();
        let game = b.push(ROOT, OsStr::new("game"), dir()).unwrap();
        b.push(game, OsStr::new("x.uproject"), file()).unwrap();
        let saved = b.push(game, OsStr::new("Saved"), dir()).unwrap();
        let inter = b.push(game, OsStr::new("Intermediate"), dir()).unwrap();
        let mut tree = b.finish(Duration::ZERO);
        crate::detect_junk(&mut tree);
        (tree, [cache, pip, notes, saved, inter])
    }

    fn home() -> PathBuf {
        std::env::temp_dir().join("disksieve-advisor-home")
    }

    #[test]
    fn home_rules_match_in_a_tree() {
        let home = home();
        let (tree, [cache, pip, ..]) = home_tree(&home);
        let advisor = Advisor::new(Linux, Some(&home));
        let guard = Guard::new(None, []);
        for (id, expected) in [(cache, "linux_user_cache"), (pip, "pip_cache")] {
            let a = advisor.advise(&tree, id, &guard).unwrap();
            assert_eq!(a.rule, Some(expected));
            assert_eq!(a.level, SafetyLevel::Safe);
            assert_eq!(a.protected, None);
        }
    }

    #[test]
    fn dev_junk_gets_its_rebuild_hint_and_unreal_saved_needs_review() {
        let home = home();
        let (tree, [.., saved, inter]) = home_tree(&home);
        let advisor = Advisor::new(Platform::CURRENT, None);
        let guard = Guard::new(None, []);
        let i = advisor.advise(&tree, inter, &guard).unwrap();
        assert_eq!((i.rule, i.level), (Some(DEV_JUNK_RULE), SafetyLevel::Safe));
        assert_eq!(i.description, JunkKind::UnrealCache.rebuild_hint());
        let s = advisor.advise(&tree, saved, &guard).unwrap();
        assert_eq!(
            (s.rule, s.level),
            (Some(DEV_JUNK_RULE), SafetyLevel::Review)
        );
    }

    #[test]
    fn unknown_items_need_review() {
        let home = home();
        let (tree, [.., notes, _, _]) = home_tree(&home);
        let a = Advisor::new(Platform::CURRENT, None)
            .advise(&tree, notes, &Guard::new(None, []))
            .unwrap();
        assert_eq!(
            (a.rule, a.level, a.protected),
            (None, SafetyLevel::Review, None)
        );
    }

    #[test]
    fn guard_refusal_makes_the_verdict_system() {
        let home = home();
        let (tree, [cache, pip, ..]) = home_tree(&home);
        let advisor = Advisor::new(Linux, Some(&home));
        // DiskSieve's own folder: the cache rules still explain it, but deleting is refused.
        let guard = Guard::new(None, [home.join(".cache")]);
        for id in [cache, pip] {
            let a = advisor.advise(&tree, id, &guard).unwrap();
            assert_eq!(a.level, SafetyLevel::System);
            assert_eq!(a.protected, Some(Refusal::AppDirectory));
            assert!(a.rule.is_some());
        }
        // The scan root itself is never moved to Trash.
        let root = advisor.advise(&tree, ROOT, &guard).unwrap();
        assert_eq!(root.protected, Some(Refusal::ScanRoot));
        assert_eq!(root.title, "Protected by DiskSieve");
    }

    #[test]
    fn system_rule_without_guard_refusal_is_review() {
        let base = Advice {
            level: SafetyLevel::System,
            rule: Some("winsxs"),
            title: "t",
            description: "d",
            alternative: None,
            protected: None,
        };
        assert_eq!(combine(base.clone(), None).level, SafetyLevel::Review);
        let refused = combine(base, Some(Refusal::SystemLocation));
        assert_eq!(refused.level, SafetyLevel::System);
        assert_eq!(refused.title, "t");
    }

    #[test]
    fn invalid_node_is_an_error() {
        let (tree, _) = home_tree(&home());
        assert!(Advisor::new(Linux, None)
            .advise(&tree, 999, &Guard::new(None, []))
            .is_err());
    }
}
