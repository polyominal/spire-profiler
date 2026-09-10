//! ABI conformance between the production C# sources and the Rust core: every
//! GetExport binding must match its `extern "C" fn` parameters and return,
//! compared as canonical classes (int / long / ulong / double / string / void).
//! Returns support only scalars and void. Bound delegates accept whitespace and
//! line-comment prefixes after a declaration boundary. The exact Cdecl delegate
//! attribute is supported; other delegate attributes, trailing block comments,
//! directives, and extra modifiers are rejected.
//! Parameter attributes remain supported. The
//! check runs inside build and fails it on mismatch. Only *bound* exports
//! are checked; test-only exports are ignored. The scanners are deliberately
//! simple (no regex) because the edge cases — nested parens in
//! [MarshalAs(...)] attributes, attribute stripping, whitespace — are the
//! point. A C# `string` parameter must also carry
//! [MarshalAs(UnmanagedType.LPUTF8Str)]: P/Invoke's default string
//! marshaling is ANSI, and the core always receives UTF-8.

use std::collections::HashMap;

use anyhow::Result;

use crate::{shim, workspace_root};

const WHITESPACE: &[char] = &[' ', '\t', '\n', '\r', '\x0b', '\x0c'];

#[derive(PartialEq, Eq)]
struct Signature {
    parameters: Vec<String>,
    returns: &'static str,
}

struct Declaration {
    signature: Signature,
    source: String,
}

struct Binding {
    delegate: String,
    export_name: String,
    source: String,
}

pub fn run() -> Result<()> {
    let root = workspace_root();
    let rust_text = std::fs::read_to_string(root.join("profiler-core/src/abi.rs"))
        .map_err(|e| anyhow::anyhow!("reading profiler-core/src/abi.rs: {e}"))?;
    let sources = shim::ProjectKind::Mod.read_sources()?;
    let sources: Vec<_> = sources
        .iter()
        .map(|(name, text)| (*name, text.as_str()))
        .collect();
    match compare_sources(&rust_text, "profiler-core/src/abi.rs", &sources) {
        Ok(bindings) => {
            println!("check-abi: {bindings} shim bindings verified against Rust exports");
            Ok(())
        }
        Err(errors) => {
            for error in &errors {
                eprintln!("check-abi: ERROR: {error}");
            }
            Err(anyhow::anyhow!(
                "ABI check failed with {} error(s)",
                errors.len()
            ))
        }
    }
}

/// One Err entry per mismatch, in binding order.
fn compare_sources(
    rust_source: &str,
    source_name: &str,
    sources: &[(&str, &str)],
) -> Result<usize, Vec<String>> {
    let mut bindings = Vec::new();
    for (name, text) in sources {
        scan_bindings(text, name, &mut bindings).map_err(|e| vec![format!("{name}: {e}")])?;
    }
    let mut exports = HashMap::new();
    scan_rust_exports(rust_source, source_name, &bindings, &mut exports)
        .map_err(|e| vec![format!("{source_name}: {e}")])?;
    let mut delegates = HashMap::new();
    for (name, text) in sources {
        scan_delegates(text, name, &bindings, &mut delegates)
            .map_err(|e| vec![format!("{name}: {e}")])?;
    }

    let mut errors = Vec::new();
    if bindings.is_empty() {
        errors.push("no GetExport bindings were parsed from the production sources".to_owned());
    }
    for binding in &bindings {
        match exports.get(&binding.export_name) {
            Some(export) => match delegates.get(&binding.delegate) {
                Some(delegate) if delegate.signature == export.signature => {}
                Some(delegate) => {
                    errors.push(format!(
                        "{}: Rust({}) -> {} [{}] != C# {}({}) -> {} [{}]; binding in {}",
                        binding.export_name,
                        export.signature.parameters.join(", "),
                        export.signature.returns,
                        export.source,
                        binding.delegate,
                        delegate.signature.parameters.join(", "),
                        delegate.signature.returns,
                        delegate.source,
                        binding.source,
                    ));
                }
                None => {
                    errors.push(format!(
                        "{}: delegate '{}' (bound to '{}') not found in the production sources",
                        binding.source, binding.delegate, binding.export_name,
                    ));
                }
            },
            None => {
                errors.push(format!(
                    "{}: '{}' is bound in the shim but has no Rust export",
                    binding.source, binding.export_name,
                ));
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(bindings.len())
}

fn map_rust_type(abi_type: &str) -> Result<String, String> {
    match abi_type {
        "i32" => Ok("int".to_owned()),
        "i64" => Ok("long".to_owned()),
        "u64" => Ok("ulong".to_owned()),
        "f64" => Ok("double".to_owned()),
        "*const c_char" => Ok("string".to_owned()),
        _ => Err(format!("unsupported Rust parameter type '{abi_type}'")),
    }
}

fn map_cs_type(abi_type: &str) -> Option<&'static str> {
    match abi_type {
        "void" => Some("void"),
        "int" => Some("int"),
        "long" => Some("long"),
        "ulong" => Some("ulong"),
        "double" => Some("double"),
        "string" => Some("string"),
        _ => None,
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
        classes.push(map_rust_type(param_type)?);
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
        classes.push(
            map_cs_type(cs_type)
                .filter(|class| *class != "void")
                .ok_or_else(|| format!("unsupported C# parameter type '{cs_type}'"))?
                .to_owned(),
        );
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
    bindings: &[Binding],
    exports: &mut HashMap<String, Declaration>,
) -> Result<(), String> {
    const NEEDLE: &str = "extern \"C\" fn ";
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
        if text[j..].starts_with('(') && bindings.iter().any(|binding| binding.export_name == name)
        {
            let raw = extract_params(text, j).map_err(|error| format!("{name}: {error}"))?;
            let after_params = j + raw.len() + 2;
            let tail = text[after_params..].trim_start_matches(WHITESPACE);
            let returns = if tail.starts_with('{') {
                "void"
            } else if let Some(after_arrow) = tail.strip_prefix("->") {
                let body_start = after_arrow
                    .find('{')
                    .ok_or_else(|| format!("{name}: Rust return type has no function body"))?;
                let return_type = after_arrow[..body_start].trim_matches(WHITESPACE);
                match return_type {
                    "i32" => "int",
                    "i64" => "long",
                    "u64" => "ulong",
                    "f64" => "double",
                    unit if unit
                        .strip_prefix('(')
                        .and_then(|inner| inner.strip_suffix(')'))
                        .is_some_and(|inner| inner.trim_matches(WHITESPACE).is_empty()) =>
                    {
                        "void"
                    }
                    _ => {
                        return Err(format!(
                            "{name}: unsupported Rust return type '{return_type}'"
                        ));
                    }
                }
            } else {
                return Err(format!(
                    "{name}: expected '{{' or '->' after Rust parameters"
                ));
            };
            exports.insert(
                name.to_owned(),
                Declaration {
                    signature: Signature {
                        parameters: rust_param_classes(raw.trim_matches(WHITESPACE))
                            .map_err(|error| format!("{name}: {error}"))?,
                        returns,
                    },
                    source: source.to_owned(),
                },
            );
            pos = after_params;
            continue;
        }
        pos = found + 1;
    }
    Ok(())
}

fn scan_delegates(
    text: &str,
    source: &str,
    bindings: &[Binding],
    delegates: &mut HashMap<String, Declaration>,
) -> Result<(), String> {
    const NEEDLE: &str = "private delegate ";
    let mut pos = 0;
    while let Some(found) = text[pos..].find(NEEDLE).map(|i| pos + i) {
        let header_start = found + NEEDLE.len();
        let open = text[header_start..]
            .find('(')
            .map(|index| header_start + index)
            .ok_or_else(|| format!("delegate at offset {found} has no parameters"))?;
        let header = text[header_start..open].trim_matches(WHITESPACE);
        let name = header
            .split_ascii_whitespace()
            .next_back()
            .unwrap_or_default();
        if !bindings.iter().any(|binding| binding.delegate == name) {
            pos = found + NEEDLE.len();
            continue;
        }
        let return_type = header[..header.len() - name.len()].trim_matches(WHITESPACE);
        let returns = map_cs_type(return_type)
            .filter(|class| *class != "string")
            .ok_or_else(|| format!("{name}: unsupported C# return type '{return_type}'"))?;
        // Only recognized declaration boundaries prove that no attached attribute remains.
        let mut prefix = text[..found].trim_end_matches(WHITESPACE);
        let mut has_cdecl = false;
        loop {
            let line = prefix.rsplit(['\r', '\n']).next().unwrap_or_default();
            let is_cdecl = line.trim_matches(WHITESPACE)
                == "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]";
            if !has_cdecl && is_cdecl {
                prefix = prefix[..prefix.len() - line.len()].trim_end_matches(WHITESPACE);
                has_cdecl = true;
                continue;
            }
            let unclassified = line.trim_start_matches(WHITESPACE).starts_with('#')
                || prefix.ends_with(']')
                || prefix.ends_with("*/");
            if !unclassified && let Some(comment) = line.rfind("//") {
                prefix = prefix[..prefix.len() - line.len() + comment].trim_end_matches(WHITESPACE);
                continue;
            }
            if unclassified || (!prefix.is_empty() && !prefix.ends_with([';', '{', '}'])) {
                return Err(format!(
                    "{name}: unsupported delegate prefix; expected a declaration boundary, \
                     whitespace, line comments, or the exact Cdecl attribute"
                ));
            }
            break;
        }
        let raw = extract_params(text, open).map_err(|error| format!("{name}: {error}"))?;
        let after_params = open + raw.len() + 2;
        let tail = text[after_params..].trim_start_matches(WHITESPACE);
        if !tail.starts_with(';') {
            return Err(format!("{name}: expected ';' after delegate parameters"));
        }
        if let Some(previous) = delegates.get(name) {
            return Err(format!(
                "{name}: ambiguous bound delegate, also declared in {}",
                previous.source
            ));
        }
        let parameters = cs_param_classes(raw.trim_matches(WHITESPACE))
            .map_err(|error| format!("{name}: {error}"))?;
        let signature = Signature {
            parameters,
            returns,
        };
        let source = source.to_owned();
        delegates.insert(name.to_owned(), Declaration { signature, source });
        pos = after_params;
    }
    Ok(())
}

/// All literal, no whitespace.
fn scan_bindings(text: &str, source: &str, bindings: &mut Vec<Binding>) -> Result<(), String> {
    const NEEDLE: &str = "GetExport<";
    const GENERIC_HELPER: &str = "GetExport<T>(IntPtr lib, string name)";
    let mut pos = 0;
    while let Some(found) = text[pos..].find(NEEDLE).map(|i| pos + i) {
        if text[found..].starts_with(GENERIC_HELPER) {
            pos = found + GENERIC_HELPER.len();
            continue;
        }
        let mut i = found + NEEDLE.len();
        let delegate_start = i;
        while text[i..].chars().next().is_some_and(is_word_char) {
            i += 1;
        }
        let delegate_name = &text[delegate_start..i];
        if text[i..].starts_with(">(lib, \"") {
            let mut j = i + ">(lib, \"".len();
            let export_start = j;
            while text[j..].chars().next().is_some_and(is_word_char) {
                j += 1;
            }
            let export_name = &text[export_start..j];
            if is_profiler_name(export_name) && text[j..].starts_with("\")") {
                bindings.push(Binding {
                    delegate: delegate_name.to_owned(),
                    export_name: export_name.to_owned(),
                    source: source.to_owned(),
                });
                pos = j + "\")".len();
                continue;
            }
        }
        return Err(format!(
            "unrecognized GetExport call at offset {found}: expected \
             GetExport<Delegate>(lib, \"spire_profiler_export\")"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare(rust: &str, name: &str, cs: &str) -> Result<usize, Vec<String>> {
        compare_sources(rust, name, &[("fixture.cs", cs)])
    }

    /// Safe and unsafe targets plus a test-only export (never GetExport'd —
    /// must be ignored).
    const GOOD_RUST: &str = r#"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_foo(amount: i32, id: *const c_char, hash: u64) {
    let _ = (amount, id, hash);
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_bar(amount: i32, started_at: i64, delta: f64) {
    let _ = (amount, started_at, delta);
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_test_reset() {
}
"#;

    /// Matching bindings (with a MarshalAs attribute to exercise the
    /// stripper) plus a private delegate that is never bound.
    const GOOD_CS: &str = r#"
internal static class ProfilerNative
{
    private delegate void NativeFoo(int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash);
    private delegate void NativeBar(int amount, long started_at, double delta);
    private delegate void NativeVoid();

    private static NativeFoo _foo;
    private static NativeBar _bar;
    private static T GetExport<T>(IntPtr lib, string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(lib, name));

    public static void Load(string libPath)
    {
        var lib = NativeLibrary.Load(libPath);
        _foo = GetExport<NativeFoo>(lib, "spire_profiler_foo");
        _bar = GetExport<NativeBar>(lib, "spire_profiler_bar");
    }
}
"#;

    #[test]
    fn known_good_binding_passes() {
        let bindings = compare(GOOD_RUST, "abi.rs", GOOD_CS).expect("good fixture must pass");
        assert_eq!(bindings, 2);
    }

    #[test]
    fn bindings_resolve_declarations_across_source_files_in_either_order() {
        let declarations = r#"
private delegate void NativeFoo(int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash);
private delegate void NativeBar(int amount, long started_at, double delta);
"#;
        let bindings = r#"
GetExport<NativeFoo>(lib, "spire_profiler_foo");
GetExport<NativeBar>(lib, "spire_profiler_bar");
"#;
        for sources in [
            [("Declarations.cs", declarations), ("Bindings.cs", bindings)],
            [("Bindings.cs", bindings), ("Declarations.cs", declarations)],
        ] {
            assert_eq!(compare_sources(GOOD_RUST, "abi.rs", &sources), Ok(2));
        }
        for (original, replacement, diagnostic) in [
            (
                "void NativeFoo",
                "long NativeFoo",
                "-> long [Declarations.cs]; binding in Bindings.cs",
            ),
            (
                "int amount",
                "double amount",
                "C# NativeFoo(double, string, ulong)",
            ),
            ("LPUTF8Str", "LPStr", "Declarations.cs: NativeFoo:"),
            (
                "private delegate void NativeFoo",
                "[return: MarshalAs(UnmanagedType.I4)]\nprivate delegate void NativeFoo",
                "Declarations.cs: NativeFoo: unsupported delegate prefix",
            ),
        ] {
            let changed = declarations.replace(original, replacement);
            let errors = compare_sources(
                GOOD_RUST,
                "abi.rs",
                &[("Bindings.cs", bindings), ("Declarations.cs", &changed)],
            )
            .expect_err("split declarations retain signature and marshalling validation");
            assert!(errors[0].contains(diagnostic), "{errors:?}");
        }
    }

    #[test]
    fn separate_sources_report_missing_and_ambiguous_declarations() {
        let binding = "GetExport<NativeBar>(lib, \"spire_profiler_bar\");";
        let declaration =
            "private delegate void NativeBar(int amount, long started_at, double delta);";
        assert_eq!(
            compare_sources(GOOD_RUST, "abi.rs", &[("Bindings.cs", binding)]),
            Err(vec![
                "Bindings.cs: delegate 'NativeBar' (bound to 'spire_profiler_bar') not found in the production sources".to_owned()
            ])
        );
        assert_eq!(
            compare_sources(
                GOOD_RUST,
                "abi.rs",
                &[
                    ("Bindings.cs", binding),
                    ("First.cs", declaration),
                    ("Second.cs", declaration),
                ],
            ),
            Err(vec![
                "Second.cs: NativeBar: ambiguous bound delegate, also declared in First.cs"
                    .to_owned()
            ])
        );
    }

    #[test]
    fn malformed_binding_in_another_source_fails_with_its_filename() {
        let errors = compare_sources(
            GOOD_RUST,
            "abi.rs",
            &[
                ("Valid.cs", GOOD_CS),
                (
                    "Broken.cs",
                    "GetExport<NativeBar>(other, \"spire_profiler_bar\");",
                ),
            ],
        )
        .expect_err("valid bindings cannot mask a malformed call in another input");
        assert!(errors[0].starts_with("Broken.cs: unrecognized GetExport call at offset 0:"));
    }

    fn compare_returns(rust_return: &str, cs_return: &str) -> Result<usize, Vec<String>> {
        let rust = format!(
            "pub extern \"C\" fn spire_profiler_value(value: i32) {rust_return} {{ todo!() }}"
        );
        let cs = format!(
            "private delegate {cs_return} NativeValue(int value);\n\
             GetExport<NativeValue>(lib, \"spire_profiler_value\");"
        );
        compare(&rust, "abi.rs", &cs)
    }

    #[test]
    fn scalar_and_unit_returns_match() {
        for (rust_return, cs_return) in [
            ("", "void"),
            ("-> ()", "void"),
            ("-> ( \n )", "void"),
            ("-> i32", "int"),
            ("-> i64", "long"),
            ("-> u64", "ulong"),
            ("->\n f64", "double"),
        ] {
            assert_eq!(compare_returns(rust_return, cs_return), Ok(1));
        }
    }

    #[test]
    fn identical_parameters_with_different_returns_fail() {
        for (rust_return, cs_return, expected_rust) in [
            ("-> u64", "long", "ulong"),
            ("-> u64", "void", "ulong"),
            ("", "ulong", "void"),
            ("-> i32", "double", "int"),
        ] {
            let errors = compare_returns(rust_return, cs_return)
                .expect_err("a return mismatch must fail despite matching parameters");
            assert_eq!(
                errors,
                [format!(
                    "spire_profiler_value: Rust(int) -> {expected_rust} [abi.rs] \
                     != C# NativeValue(int) -> {cs_return} [fixture.cs]; binding in fixture.cs"
                )]
            );
        }
    }

    #[test]
    fn unsupported_and_malformed_returns_fail_closed() {
        for (rust_return, cs_return, diagnostic) in [
            ("-> bool", "bool", "unsupported Rust return type 'bool'"),
            (
                "-> Custom",
                "Custom",
                "unsupported Rust return type 'Custom'",
            ),
            (
                "-> *const c_char",
                "string",
                "unsupported Rust return type '*const c_char'",
            ),
            ("-> u64", "string", "unsupported C# return type 'string'"),
            ("-> u64", "bool", "unsupported C# return type 'bool'"),
            ("-> u64", "Custom", "unsupported C# return type 'Custom'"),
            ("-> u64", "ulong[]", "unsupported C# return type 'ulong[]'"),
            (
                "-> u64",
                "ref ulong",
                "unsupported C# return type 'ref ulong'",
            ),
            ("-> u64", "", "unsupported C# return type ''"),
            ("->", "void", "unsupported Rust return type ''"),
            (
                "-> () -> ()",
                "void",
                "unsupported Rust return type '() -> ()'",
            ),
            ("u64", "ulong", "expected '{' or '->' after Rust parameters"),
        ] {
            let errors = compare_returns(rust_return, cs_return)
                .expect_err("unrecognized return syntax must never compare equal");
            assert_eq!(errors.len(), 1);
            assert!(
                errors[0].contains(diagnostic),
                "unexpected error: {errors:?}"
            );
        }
    }

    #[test]
    fn signatures_without_required_terminators_fail() {
        let rust = "pub extern \"C\" fn spire_profiler_foo() -> u64";
        let errors = compare(rust, "abi.rs", GOOD_CS)
            .expect_err("a return type without a function body must fail");
        assert_eq!(
            errors,
            ["abi.rs: spire_profiler_foo: Rust return type has no function body"]
        );
        let cs = GOOD_CS.replace("double delta);", "double delta) extra;");
        let errors = compare(GOOD_RUST, "abi.rs", &cs)
            .expect_err("a malformed delegate terminator must fail");
        assert_eq!(
            errors,
            ["fixture.cs: NativeBar: expected ';' after delegate parameters"]
        );
    }

    #[test]
    fn delegate_attributes_and_unclassified_prefixes_are_rejected() {
        let rust = "pub extern \"C\" fn spire_profiler_value(value: i32) -> i64 { 0 }";
        for prefix in [
            "[return: MarshalAs(UnmanagedType.I4)]",
            "[ return \n : MarshalAs(UnmanagedType.I4)]",
            "[return: MarshalAs(UnmanagedType.I4)]\n[UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
            "[UnmanagedFunctionPointer(CallingConvention.StdCall)]",
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl, CharSet = CharSet.Ansi)]",
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]\n[UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]\n[return: MarshalAs(UnmanagedType.I4)]",
            "[return /* native result */: MarshalAs(UnmanagedType.I4)]",
            "[return: MarshalAs(UnmanagedType.I4)] // native result; documented",
            "[return: MarshalAs(UnmanagedType.I4)] /* native result { documented */",
            "/* native result { documented */",
            "[return: MarshalAs(UnmanagedType.I4)] // native result // documented",
            "[return: MarshalAs(UnmanagedType.I4)]\n// one comment\n// another comment",
        ] {
            let cs = format!(
                "{prefix}\nprivate delegate long NativeValue(int value);\n\
                 GetExport<NativeValue>(lib, \"spire_profiler_value\");"
            );
            let errors = compare(rust, "abi.rs", &cs)
                .expect_err("unclassified prefixes can hide native return representation changes");
            assert_eq!(
                errors,
                [
                    "fixture.cs: NativeValue: unsupported delegate prefix; expected a declaration boundary, \
                 whitespace, line comments, or the exact Cdecl attribute"
                ]
            );
        }
    }

    #[test]
    fn exact_cdecl_annotation_preserves_scalar_and_utf8_checks() {
        for newline in ["\n", "\r", "\r\n"] {
            let cs = GOOD_CS.replace(
                "private delegate",
                "// native ABI\n[UnmanagedFunctionPointer(CallingConvention.Cdecl)]\n// signature\nprivate delegate",
            ).replace('\n', newline);
            assert_eq!(compare(GOOD_RUST, "abi.rs", &cs), Ok(2));
            let bad_return = cs.replace("delegate void NativeFoo", "delegate long NativeFoo");
            assert!(compare(GOOD_RUST, "abi.rs", &bad_return).is_err());
            let bad_string = cs.replace("LPUTF8Str", "LPStr");
            assert!(compare(GOOD_RUST, "abi.rs", &bad_string).is_err());
            let hidden_return = cs.replace(
                "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
                "[return: MarshalAs(UnmanagedType.I4)]\n[UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
            ).replace('\n', newline);
            assert!(compare(GOOD_RUST, "abi.rs", &hidden_return).is_err());
        }
    }

    #[test]
    fn directives_and_modifiers_cannot_hide_return_attributes() {
        let rust = "pub extern \"C\" fn spire_profiler_value(value: i32) -> i64 { 0 }";
        for newline in ["\n", "\r", "\r\n"] {
            for (prefix, suffix) in [
                ("new ", ""),
                ("#pragma warning disable\n", ""),
                ("#region native result ;\n", "\n#endregion"),
                ("#region native result {\n", "\n#endregion"),
                ("#region native result } // documented\n", "\n#endregion"),
            ] {
                let cs = format!(
                    "[return: MarshalAs(UnmanagedType.I4)]\n\
                     {prefix}private delegate long NativeValue(int value);{suffix}\n\
                     GetExport<NativeValue>(lib, \"spire_profiler_value\");"
                )
                .replace('\n', newline);
                let errors = compare(rust, "abi.rs", &cs)
                    .expect_err("unclassified prefixes cannot prove absence of return attributes");
                assert_eq!(
                    errors,
                    [
                        "fixture.cs: NativeValue: unsupported delegate prefix; expected a declaration boundary, \
                     whitespace, line comments, or the exact Cdecl attribute"
                    ]
                );
            }
        }
    }

    #[test]
    fn line_comments_before_bound_delegates_are_accepted() {
        for newline in ["\n", "\r", "\r\n"] {
            let cs = GOOD_CS
                .replace(
                    "private delegate void NativeFoo",
                    "// Native declaration; documented here\n// More documentation\n\
                 private delegate void NativeFoo",
                )
                .replace('\n', newline);
            assert_eq!(compare(GOOD_RUST, "abi.rs", &cs), Ok(2));
        }
    }

    #[test]
    fn unsupported_unbound_signatures_are_ignored() {
        let rust = format!(
            "{GOOD_RUST}\n\
             pub extern \"C\" fn spire_profiler_unused(value: Custom) -> Custom {{ value }}"
        );
        let cs = format!(
            "[return: MarshalAs(UnmanagedType.I4)] /* unbound attribute */\n\
             private delegate Custom NativeUnused(Custom value);\n{GOOD_CS}"
        );
        assert_eq!(compare(&rust, "abi.rs", &cs), Ok(2));
    }

    #[test]
    fn identically_spelled_unsupported_parameters_do_not_match() {
        let rust = GOOD_RUST.replace("amount: i32", "amount: Custom");
        let cs = GOOD_CS.replace("int amount", "Custom amount");
        let errors = compare(&rust, "abi.rs", &cs)
            .expect_err("unknown parameter classes must not match by spelling");
        assert!(errors[0].contains("unsupported Rust parameter type 'Custom'"));
        let errors = compare(GOOD_RUST, "abi.rs", &cs)
            .expect_err("unknown C# parameter classes must fail explicitly");
        assert!(errors[0].contains("unsupported C# parameter type 'Custom'"));
    }

    /// Drifted delegate parameter order must fail with a side-by-side diff.
    #[test]
    fn shifted_parameter_list_fails_with_a_diff() {
        for (original, shifted, expected) in [
            (
                "int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash",
                "[MarshalAs(UnmanagedType.LPUTF8Str)] string id, int amount, ulong hash",
                "spire_profiler_foo: Rust(int, string, ulong) -> void [abi.rs] \
                 != C# NativeFoo(string, int, ulong) -> void [fixture.cs]; binding in fixture.cs",
            ),
            (
                "int amount, long started_at, double delta",
                "long started_at, int amount, double delta",
                "spire_profiler_bar: Rust(int, long, double) -> void [abi.rs] \
                 != C# NativeBar(long, int, double) -> void [fixture.cs]; binding in fixture.cs",
            ),
        ] {
            let cs = GOOD_CS.replace(original, shifted);
            let errors = compare(GOOD_RUST, "abi.rs", &cs)
                .expect_err("shifted params must fail for safe and unsafe exports");
            assert_eq!(errors, [expected]);
        }
    }

    /// A bound name with no Rust export resolves to a null delegate at load.
    #[test]
    fn binding_without_an_export_fails() {
        for name in ["spire_profiler_foo", "spire_profiler_bar"] {
            let cs = GOOD_CS.replace(name, "spire_profiler_missing");
            let errors = compare(GOOD_RUST, "abi.rs", &cs).expect_err("missing export must fail");
            assert_eq!(
                errors,
                [
                    "fixture.cs: 'spire_profiler_missing' is bound in the shim but has no Rust export"
                ]
            );
        }
    }

    #[test]
    fn empty_cs_fails_rather_than_passing_vacuously() {
        let errors = compare(GOOD_RUST, "abi.rs", "").expect_err("zero bindings must fail");
        assert_eq!(
            errors,
            ["no GetExport bindings were parsed from the production sources"]
        );
    }

    #[test]
    fn malformed_binding_fails_alongside_a_valid_binding() {
        let cs = GOOD_CS.replace(
            "    private static NativeFoo _foo;",
            "    private static NativeFoo _foo;\n    private static NativeVoid _bad;",
        ) + "\n_bad = GetExport<NativeVoid>(IntPtr lib, \"spire_profiler_test_reset\");\n";
        let errors = compare(GOOD_RUST, "abi.rs", &cs).expect_err("malformed call must fail");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("unrecognized GetExport call"),
            "unexpected error: {}",
            errors[0]
        );
    }

    #[test]
    fn unbalanced_params_are_a_scanner_error() {
        let cs = r#"
private delegate void NativeFoo(int amount;
GetExport<NativeFoo>(lib, "spire_profiler_foo");
"#;
        let errors = compare(GOOD_RUST, "abi.rs", cs).expect_err("unbalanced parens must fail");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unbalanced parens"));
    }

    /// A bare string reverts marshaling to ANSI, corrupting non-ASCII ids.
    #[test]
    fn string_param_without_lputf8str_fails() {
        let cs = GOOD_CS.replace(
            "[MarshalAs(UnmanagedType.LPUTF8Str)] string id",
            "string id",
        );
        let errors = compare(GOOD_RUST, "abi.rs", &cs).expect_err("bare string must fail");
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
        let cs = GOOD_CS.replace(
            "[MarshalAs(UnmanagedType.LPUTF8Str)] string id",
            "[MarshalAs(UnmanagedType.LPStr)] string id",
        );
        let errors = compare(GOOD_RUST, "abi.rs", &cs).expect_err("LPStr must fail");
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
