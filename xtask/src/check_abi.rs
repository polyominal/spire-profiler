//! ABI conformance between the generated C# shim and the Rust core: every
//! GetExport binding must match its `unsafe extern "C" fn` parameter list,
//! compared as canonical classes (int / long / ulong / double / string). The
//! check runs inside build and fails it on mismatch. Only *bound* exports
//! are checked; test-only exports are ignored. The scanners are deliberately
//! simple (no regex) because the edge cases — nested parens in
//! [MarshalAs(...)] attributes, attribute stripping, whitespace — are the
//! point. A C# `string` parameter must also carry
//! [MarshalAs(UnmanagedType.LPUTF8Str)]: P/Invoke's default string
//! marshaling is ANSI, and the core always receives UTF-8.

use std::collections::HashMap;

use anyhow::Result;

use crate::workspace_root;

const WHITESPACE: &[char] = &[' ', '\t', '\n', '\r', '\x0b', '\x0c'];

struct RustExport {
    classes: Vec<String>,
    source: String,
}

struct Binding {
    delegate: String,
    export_name: String,
}

#[derive(Debug)]
pub struct Report {
    pub bindings: usize,
}

pub fn run() -> Result<()> {
    let root = workspace_root();
    let rust_text = std::fs::read_to_string(root.join("profiler-core/src/abi.rs"))
        .map_err(|e| anyhow::anyhow!("reading profiler-core/src/abi.rs: {e}"))?;
    let template = std::fs::read_to_string(root.join("shim/shim.cs.template"))
        .map_err(|e| anyhow::anyhow!("reading shim/shim.cs.template: {e}"))?;
    match compare(&rust_text, "abi.rs", &template) {
        Ok(report) => {
            println!(
                "check_abi: {} shim bindings verified against Rust exports",
                report.bindings
            );
            Ok(())
        }
        Err(errors) => {
            for error in &errors {
                eprintln!("check_abi: ERROR: {error}");
            }
            Err(anyhow::anyhow!(
                "ABI check failed with {} error(s)",
                errors.len()
            ))
        }
    }
}

/// One Err entry per mismatch, in binding order.
fn compare(rust_source: &str, source_name: &str, template: &str) -> Result<Report, Vec<String>> {
    let mut exports = HashMap::new();
    scan_rust_exports(rust_source, source_name, &mut exports).map_err(|e| vec![e])?;

    let mut delegates = HashMap::new();
    scan_delegates(template, &mut delegates).map_err(|e| vec![e])?;
    let mut bindings = Vec::new();
    scan_bindings(template, &mut bindings).map_err(|e| vec![e])?;

    let mut errors = Vec::new();
    for binding in &bindings {
        match exports.get(&binding.export_name) {
            Some(export) => match delegates.get(&binding.delegate) {
                Some(classes) if *classes == export.classes => {}
                Some(classes) => {
                    errors.push(format!(
                        "{}: Rust({}) [{}] != C# {}({})",
                        binding.export_name,
                        export.classes.join(", "),
                        export.source,
                        binding.delegate,
                        classes.join(", "),
                    ));
                }
                None => {
                    errors.push(format!(
                        "delegate '{}' (bound to '{}') not found in the template",
                        binding.delegate, binding.export_name,
                    ));
                }
            },
            None => {
                errors.push(format!(
                    "'{}' is bound in the shim but has no Rust export",
                    binding.export_name,
                ));
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Report {
        bindings: bindings.len(),
    })
}

/// Anything unlisted surfaces as `?<type>` and mismatches any export.
fn map_rust_type(abi_type: &str) -> String {
    match abi_type {
        "i32" => "int".to_owned(),
        "i64" => "long".to_owned(),
        "u64" => "ulong".to_owned(),
        "f64" => "double".to_owned(),
        "*const c_char" => "string".to_owned(),
        _ => format!("?{abi_type}"),
    }
}

fn map_cs_type(abi_type: &str) -> String {
    match abi_type {
        "int" | "long" | "ulong" | "double" | "string" => abi_type.to_owned(),
        _ => format!("?{abi_type}"),
    }
}

fn is_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// `_profiler_` must sit strictly inside the identifier.
fn is_profiler_name(name: &str) -> bool {
    const SEP: &str = "_profiler_";
    match name.find(SEP) {
        Some(separator_index) => separator_index > 0 && separator_index + SEP.len() < name.len(),
        None => false,
    }
}

/// Balances nested parens (MarshalAs attributes contain parens, so a regex
/// cannot do this); unbalanced input fails the build.
fn extract_params(text: &str, open: usize) -> Result<&str, String> {
    let mut depth = 0usize;
    for (index, character) in text[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&text[open + 1..open + index]);
                }
            }
            _ => {}
        }
    }
    Err("unbalanced parens while extracting parameters".to_owned())
}

/// The first colon is always the name/type separator.
fn rust_param_classes(raw: &str) -> Result<Vec<String>, String> {
    let mut classes = Vec::new();
    for param in raw.split(',') {
        let part = param.trim_matches(WHITESPACE);
        if part.is_empty() {
            continue;
        }
        let colon = part
            .find(':')
            .ok_or_else(|| format!("param '{part}' has no name/type ':' separator"))?;
        let param_type = part[colon + 1..].trim_matches(WHITESPACE);
        classes.push(map_rust_type(param_type));
    }
    Ok(classes)
}

/// P/Invoke's default string marshaling is ANSI, which silently corrupts
/// non-ASCII text before the core sees it.
fn cs_param_classes(raw: &str) -> Result<Vec<String>, String> {
    let mut classes = Vec::new();
    for param in raw.split(',') {
        let original = param.trim_matches(WHITESPACE);
        if original.is_empty() {
            continue;
        }
        let part = strip_attributes(original)?;
        let type_end = part
            .find(|character: char| character.is_ascii_whitespace())
            .unwrap_or(part.len());
        if type_end == 0 {
            return Err(format!("param '{original}' has no type token"));
        }
        let cs_type = &part[..type_end];
        if cs_type == "string" && !original.contains("[MarshalAs(UnmanagedType.LPUTF8Str)]") {
            return Err(format!(
                "param '{original}' is a string without \
                 [MarshalAs(UnmanagedType.LPUTF8Str)] (the ABI requires UTF-8 marshaling)"
            ));
        }
        classes.push(map_cs_type(cs_type));
    }
    Ok(classes)
}

/// Removes `[...]` attribute groups with bracket balancing.
fn strip_attributes(text: &str) -> Result<&str, String> {
    match text.find('[') {
        Some(start) => {
            let mut depth = 0usize;
            let mut close = None;
            for (index, character) in text[start..].char_indices() {
                match character {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(start + index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else {
                return Err(format!("unbalanced attribute brackets in param '{text}'"));
            };
            let mut after = &text[close + 1..];
            while after
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_whitespace())
            {
                after = &after[1..];
            }
            strip_attributes(after)
        }
        None => Ok(text),
    }
}

/// Later occurrences overwrite earlier ones.
fn scan_rust_exports(
    text: &str,
    source: &str,
    exports: &mut HashMap<String, RustExport>,
) -> Result<(), String> {
    const NEEDLE: &str = "unsafe extern \"C\" fn ";
    let mut pos = 0;
    while let Some(found) = text[pos..].find(NEEDLE).map(|i| pos + i) {
        let mut i = found + NEEDLE.len();
        let name_start = i;
        while text[i..].chars().next().is_some_and(is_word_char) {
            i += 1;
        }
        let name = &text[name_start..i];
        let mut j = i;
        while text[j..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_whitespace())
        {
            j += 1;
        }
        if text[j..].starts_with('(') && is_profiler_name(name) {
            let raw = extract_params(text, j)?;
            exports.insert(
                name.to_owned(),
                RustExport {
                    classes: rust_param_classes(raw.trim_matches(WHITESPACE))?,
                    source: source.to_owned(),
                },
            );
            pos = j + 1;
            continue;
        }
        pos = found + 1;
    }
    Ok(())
}

/// The open paren must follow the name immediately.
fn scan_delegates(
    template: &str,
    delegates: &mut HashMap<String, Vec<String>>,
) -> Result<(), String> {
    const NEEDLE: &str = "private delegate void ";
    let mut pos = 0;
    while let Some(found) = template[pos..].find(NEEDLE).map(|i| pos + i) {
        let mut i = found + NEEDLE.len();
        let name_start = i;
        while template[i..].chars().next().is_some_and(is_word_char) {
            i += 1;
        }
        let name = &template[name_start..i];
        if template[i..].starts_with('(') {
            let raw = extract_params(template, i)?;
            delegates.insert(
                name.to_owned(),
                cs_param_classes(raw.trim_matches(WHITESPACE))?,
            );
            pos = i + 1;
            continue;
        }
        pos = found + 1;
    }
    Ok(())
}

/// All literal, no whitespace.
fn scan_bindings(template: &str, bindings: &mut Vec<Binding>) -> Result<(), String> {
    const NEEDLE: &str = "GetExport<";
    let mut pos = 0;
    while let Some(found) = template[pos..].find(NEEDLE).map(|i| pos + i) {
        let mut i = found + NEEDLE.len();
        let delegate_start = i;
        while template[i..].chars().next().is_some_and(is_word_char) {
            i += 1;
        }
        let delegate_name = &template[delegate_start..i];
        if template[i..].starts_with(">(lib, \"") {
            let mut j = i + ">(lib, \"".len();
            let export_start = j;
            while template[j..].chars().next().is_some_and(is_word_char) {
                j += 1;
            }
            let export_name = &template[export_start..j];
            if is_profiler_name(export_name) && template[j..].starts_with("\")") {
                bindings.push(Binding {
                    delegate: delegate_name.to_owned(),
                    export_name: export_name.to_owned(),
                });
                pos = j + "\")".len();
                continue;
            }
        }
        pos = found + 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known-good target plus a test-only export (never GetExport'd —
    /// must be ignored).
    const GOOD_RUST: &str = r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_foo(amount: i32, id: *const c_char, hash: u64) {
    let _ = (amount, id, hash);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_test_reset() {
}
"#;

    /// One matching binding (with a MarshalAs attribute to exercise the
    /// stripper) plus a private delegate that is never bound.
    const GOOD_TMPL: &str = r#"
internal static class ProfilerNative
{
    private delegate void NativeFoo(int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash);
    private delegate void NativeVoid();

    private static NativeFoo _foo;

    public static void Load(string libPath)
    {
        var lib = NativeLibrary.Load(libPath);
        _foo = GetExport<NativeFoo>(lib, "spire_profiler_foo");
    }
}
"#;

    #[test]
    fn known_good_binding_passes() {
        let report = compare(GOOD_RUST, "abi.rs", GOOD_TMPL).expect("good fixture must pass");
        assert_eq!(report.bindings, 1);
    }

    /// Drifted delegate parameter order must fail with a side-by-side diff.
    #[test]
    fn shifted_parameter_list_fails_with_a_diff() {
        let template = GOOD_TMPL.replace(
            "int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash",
            "[MarshalAs(UnmanagedType.LPUTF8Str)] string id, int amount, ulong hash",
        );
        let errors = compare(GOOD_RUST, "abi.rs", &template).expect_err("shifted params must fail");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "spire_profiler_foo: Rust(int, string, ulong) [abi.rs] \
             != C# NativeFoo(string, int, ulong)"
        );
    }

    /// A bound name with no Rust export resolves to a null delegate at load.
    #[test]
    fn binding_without_an_export_fails() {
        let template = GOOD_TMPL.replace("spire_profiler_foo", "spire_profiler_missing");
        let errors = compare(GOOD_RUST, "abi.rs", &template).expect_err("missing export must fail");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "'spire_profiler_missing' is bound in the shim but has no Rust export"
        );
    }

    /// Never a vacuous zero-binding pass.
    #[test]
    fn unbalanced_params_are_a_scanner_error() {
        let template = "private delegate void NativeFoo(int amount;\n";
        let errors =
            compare(GOOD_RUST, "abi.rs", template).expect_err("unbalanced parens must fail");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unbalanced parens"));
    }

    /// A bare string reverts marshaling to ANSI, corrupting non-ASCII ids.
    #[test]
    fn string_param_without_lputf8str_fails() {
        let template = GOOD_TMPL.replace(
            "[MarshalAs(UnmanagedType.LPUTF8Str)] string id",
            "string id",
        );
        let errors = compare(GOOD_RUST, "abi.rs", &template).expect_err("bare string must fail");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("without [MarshalAs(UnmanagedType.LPUTF8Str)]"),
            "unexpected error: {}",
            errors[0]
        );
    }

    /// LPStr is exactly the ANSI default the requirement exists to forbid.
    #[test]
    fn string_param_with_a_non_utf8_marshalas_kind_fails() {
        let template = GOOD_TMPL.replace(
            "[MarshalAs(UnmanagedType.LPUTF8Str)] string id",
            "[MarshalAs(UnmanagedType.LPStr)] string id",
        );
        let errors = compare(GOOD_RUST, "abi.rs", &template).expect_err("LPStr must fail");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("without [MarshalAs(UnmanagedType.LPUTF8Str)]"),
            "unexpected error: {}",
            errors[0]
        );
    }

    /// A bare `_profiler_` in an unrelated identifier must not masquerade
    /// as an export name.
    #[test]
    fn profiler_name_pattern_rejects_boundary_separators() {
        assert!(is_profiler_name("spire_profiler_init"));
        assert!(!is_profiler_name("_profiler_foo"), "leading sep fails");
        assert!(!is_profiler_name("foo_profiler_"), "trailing sep fails");
        assert!(!is_profiler_name("foo_bar"));
    }
}
