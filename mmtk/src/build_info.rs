#[cfg(not(feature = "g3_compatibilities"))]
mod raw {
    // The include imports a full list of the constants in built.rs from https://docs.rs/built/latest/built/index.html
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[cfg(feature = "g3_compatibilities")]
mod raw {
    pub const PKG_VERSION: &str = "<unknown>";
    pub const GIT_COMMIT_HASH: Option<&str> = None;
    pub const GIT_DIRTY: Option<bool> = None;
}

lazy_static! {
    // Owned string for the binding version, such as MMTk OpenJDK 0.14.0 (cfc755f-dirty)
    static ref BINDING_VERSION_STRING: String = match (raw::GIT_COMMIT_HASH, raw::GIT_DIRTY) {
        (Some(hash), Some(dirty)) => format!("MMTk OpenJDK {} ({}{})", raw::PKG_VERSION, hash.split_at(7).0, if dirty { "-dirty" } else { "" }),
        (Some(hash), None) => format!("MMTk OpenJDK {} ({}{})", raw::PKG_VERSION, hash.split_at(7).0, "-?"),
        _ => format!("MMTk OpenJDK {}", raw::PKG_VERSION),
    };
    // Owned string for both binding and core version.
    static ref MMTK_OPENJDK_FULL_VERSION_STRING: String = format!("{}, using {}", *BINDING_VERSION_STRING, *mmtk::build_info::MMTK_FULL_BUILD_INFO);

    // Exposed &str for the full version.
    pub static ref MMTK_OPENJDK_FULL_VERSION: &'static str = &MMTK_OPENJDK_FULL_VERSION_STRING;
}
