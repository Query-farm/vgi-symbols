//! Symbol demangling: Itanium C++ (`_Z…`), Rust (legacy `_ZN…` + v0 `_R…`),
//! MSVC (`?…`), and Swift, via `symbolic-demangle`.
//!
//! Pure and stateless — no cache, no module needed. Exposed as the SQL scalar
//! `demangle(mangled, lang := 'auto')` because callers frequently arrive with
//! raw linkage names from another tool and want them readable in SQL.

use symbolic_common::{Language, Name, NameMangling};
use symbolic_demangle::{Demangle, DemangleOptions};

/// A demangling language selector (`lang` argument of `demangle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemangleLang {
    /// Detect the scheme from the mangled string.
    Auto,
    /// Itanium C++ ABI.
    Cpp,
    /// Rust (legacy + v0).
    Rust,
    /// Microsoft Visual C++.
    Msvc,
    /// Swift.
    Swift,
}

impl DemangleLang {
    /// Parse the `lang` argument (`auto`/`cpp`/`rust`/`msvc`/`swift`,
    /// case-insensitive). `c++` is accepted as an alias of `cpp`.
    pub fn parse(s: &str) -> Result<DemangleLang, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Ok(DemangleLang::Auto),
            "cpp" | "c++" | "cxx" => Ok(DemangleLang::Cpp),
            "rust" => Ok(DemangleLang::Rust),
            "msvc" => Ok(DemangleLang::Msvc),
            "swift" => Ok(DemangleLang::Swift),
            other => Err(format!(
                "unknown demangle lang '{other}' (expected auto/cpp/rust/msvc/swift)"
            )),
        }
    }

    fn language(self) -> Language {
        match self {
            DemangleLang::Auto => Language::Unknown,
            DemangleLang::Cpp => Language::Cpp,
            DemangleLang::Rust => Language::Rust,
            // MSVC mangling is the Microsoft C++ scheme; symbolic keys it under Cpp
            // and dispatches on the leading `?`. Detection handles it either way.
            DemangleLang::Msvc => Language::Cpp,
            DemangleLang::Swift => Language::Swift,
        }
    }
}

/// Demangle `mangled` under the given language selector, returning the
/// human-readable name (function name only, no parameter types). If the name is
/// not mangled (or cannot be demangled) the input is returned unchanged, so the
/// function is total and never errors on arbitrary input.
pub fn demangle(mangled: &str, lang: DemangleLang) -> String {
    let name = match lang {
        DemangleLang::Auto => Name::from(mangled),
        other => Name::new(mangled, NameMangling::Mangled, other.language()),
    };
    name.try_demangle(DemangleOptions::name_only()).into_owned()
}

/// Best-effort demangle that returns the demangled form only when it actually
/// changed (i.e. the input really was mangled). Used by the resolver to fill the
/// `function` column while keeping the raw linkage name in `function_raw`.
pub fn try_demangle(mangled: &str) -> String {
    Name::from(mangled)
        .try_demangle(DemangleOptions::name_only())
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itanium_cpp() {
        assert_eq!(demangle("_ZN3foo3barEv", DemangleLang::Auto), "foo::bar");
        assert_eq!(demangle("_ZN3foo3barEv", DemangleLang::Cpp), "foo::bar");
    }

    #[test]
    fn rust_legacy() {
        // Legacy Rust mangling.
        let out = demangle(
            "_ZN4core3fmt3num3imp7fmt_u6417h0123456789abcdefE",
            DemangleLang::Rust,
        );
        assert!(out.contains("fmt_u64"), "got {out}");
    }

    #[test]
    fn passthrough_unmangled() {
        assert_eq!(demangle("main", DemangleLang::Auto), "main");
        assert_eq!(demangle("", DemangleLang::Auto), "");
    }

    #[test]
    fn unknown_lang_rejected() {
        assert!(DemangleLang::parse("klingon").is_err());
        assert_eq!(DemangleLang::parse("C++").unwrap(), DemangleLang::Cpp);
    }
}
