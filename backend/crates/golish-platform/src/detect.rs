//! Platform / architecture detection.
//!
//! Compile-time `#[cfg]` blocks are intentionally concentrated here so
//! that the rest of the workspace can use the runtime-friendly
//! [`Platform`] facade.

use std::fmt;

/// Coarse-grained operating system classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformKind {
    MacOs,
    Linux,
    Windows,
    /// FreeBSD, NetBSD, OpenBSD, Illumos, etc. — handled with a
    /// generic-Unix branch where we have one.
    OtherUnix,
}

impl PlatformKind {
    /// Detect the kind of platform the binary was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            PlatformKind::MacOs
        } else if cfg!(target_os = "windows") {
            PlatformKind::Windows
        } else if cfg!(target_os = "linux") {
            PlatformKind::Linux
        } else if cfg!(unix) {
            PlatformKind::OtherUnix
        } else {
            PlatformKind::Linux
        }
    }

    pub const fn is_windows(self) -> bool {
        matches!(self, PlatformKind::Windows)
    }

    pub const fn is_macos(self) -> bool {
        matches!(self, PlatformKind::MacOs)
    }

    pub const fn is_linux(self) -> bool {
        matches!(self, PlatformKind::Linux)
    }

    pub const fn is_unix(self) -> bool {
        matches!(
            self,
            PlatformKind::MacOs | PlatformKind::Linux | PlatformKind::OtherUnix
        )
    }
}

impl fmt::Display for PlatformKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PlatformKind::MacOs => "macos",
            PlatformKind::Linux => "linux",
            PlatformKind::Windows => "windows",
            PlatformKind::OtherUnix => "unix",
        })
    }
}

/// CPU architecture classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    Other,
}

impl Arch {
    pub const fn current() -> Self {
        if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            Arch::Other
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
            Arch::Other => "other",
        })
    }
}

/// Public facade for the rest of the workspace.
///
/// `Platform::current()` is the entry point; from it you reach
/// well-typed accessors that delegate to the per-module helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Platform {
    pub kind: PlatformKind,
    pub arch: Arch,
}

impl Platform {
    pub const fn current() -> Self {
        Self {
            kind: PlatformKind::current(),
            arch: Arch::current(),
        }
    }

    pub const fn is_windows(self) -> bool {
        self.kind.is_windows()
    }
    pub const fn is_macos(self) -> bool {
        self.kind.is_macos()
    }
    pub const fn is_linux(self) -> bool {
        self.kind.is_linux()
    }
    pub const fn is_unix(self) -> bool {
        self.kind.is_unix()
    }

    /// File extension used for native shared libraries on this
    /// platform: `dylib` (macOS), `so` (Linux/other Unix), `dll`
    /// (Windows). Returned without a leading dot so callers can format
    /// at will.
    pub const fn shared_lib_extension(self) -> &'static str {
        match self.kind {
            PlatformKind::MacOs => "dylib",
            PlatformKind::Windows => "dll",
            _ => "so",
        }
    }

    /// File extension used for executables on this platform — `".exe"`
    /// on Windows, empty otherwise. The leading dot is included so
    /// callers can do `format!("{name}{ext}")`.
    pub const fn executable_extension(self) -> &'static str {
        if self.kind.is_windows() {
            ".exe"
        } else {
            ""
        }
    }

    /// Returns `(os, arch)` strings used by pg-embed and similar
    /// "fetch the right binary tarball" code paths.
    pub fn fetch_tag(self) -> (&'static str, &'static str) {
        let os = match self.kind {
            PlatformKind::MacOs => "darwin",
            PlatformKind::Windows => "windows",
            _ => "linux",
        };
        let arch = match self.arch {
            Arch::Aarch64 => "arm64v8",
            _ => "amd64",
        };
        (os, arch)
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_consistent() {
        let p = Platform::current();
        assert_eq!(p.kind, PlatformKind::current());
        assert_eq!(p.arch, Arch::current());
    }

    #[test]
    fn shared_lib_extension_on_current() {
        let ext = Platform::current().shared_lib_extension();
        if cfg!(target_os = "macos") {
            assert_eq!(ext, "dylib");
        } else if cfg!(target_os = "windows") {
            assert_eq!(ext, "dll");
        } else {
            assert_eq!(ext, "so");
        }
    }

    #[test]
    fn executable_extension_on_current() {
        let ext = Platform::current().executable_extension();
        if cfg!(target_os = "windows") {
            assert_eq!(ext, ".exe");
        } else {
            assert_eq!(ext, "");
        }
    }
}
