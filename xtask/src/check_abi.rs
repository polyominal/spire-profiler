//! ABI conformance compares parsed Rust exports with bound C# delegates.
//! Project policy permits only scalar returns, scalar and caller-owned buffer
//! parameters, and explicit UTF-8 string marshaling. Unbound exports are ignored.

use std::collections::HashMap;

use anyhow::Result;
use quote::ToTokens;
use serde::Deserialize;

use crate::{csharp, shim, workspace_root};

#[derive(Deserialize, PartialEq)]
struct Signature {
    parameters: Vec<String>,
    returns: String,
}

#[derive(Deserialize)]
struct Declaration {
    name: String,
    signature: Signature,
    source: String,
}

#[derive(Deserialize)]
struct Binding {
    delegate_name: String,
    export_name: String,
    source: String,
}

#[derive(Deserialize)]
struct Managed {
    bindings: Vec<Binding>,
    delegates: Vec<Declaration>,
}

pub fn run(shell: &xshell::Shell) -> Result<()> {
    let root = workspace_root();
    let rust_text = std::fs::read_to_string(root.join("profiler-core/src/abi.rs"))
        .map_err(|e| anyhow::anyhow!("reading profiler-core/src/abi.rs: {e}"))?;
    let sources = shim::Project::source(shim::ProjectKind::Mod).read_sources(shell)?;
    let sources: Vec<_> = sources
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
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

fn compare_sources(
    rust_source: &str,
    source_name: &str,
    sources: &[(&str, &str)],
) -> Result<usize, Vec<String>> {
    let managed: Managed =
        csharp::parse("abi", sources).map_err(|error| vec![error.to_string()])?;
    let exports = rust_exports(rust_source, source_name, &managed.bindings)
        .map_err(|error| vec![format!("{source_name}: {error}")])?;
    let mut delegates: HashMap<&str, &Declaration> = HashMap::new();
    for declaration in &managed.delegates {
        if let Some(previous) = delegates.insert(&declaration.name, declaration) {
            return Err(vec![format!(
                "{}: {}: ambiguous bound delegate, also declared in {}",
                declaration.source, declaration.name, previous.source
            )]);
        }
    }
    let mut errors = Vec::new();
    if managed.bindings.is_empty() {
        errors.push("no GetExport bindings were parsed from the production sources".to_owned());
    }
    for binding in &managed.bindings {
        match exports.get(binding.export_name.as_str()) {
            Some(export) => match delegates.get(binding.delegate_name.as_str()) {
                Some(delegate) if delegate.signature == export.signature => {}
                Some(delegate) => errors.push(format!(
                    "{}: Rust({}) -> {} [{}] != C# {}({}) -> {} [{}]; binding in {}",
                    binding.export_name,
                    export.signature.parameters.join(", "),
                    export.signature.returns,
                    export.source,
                    binding.delegate_name,
                    delegate.signature.parameters.join(", "),
                    delegate.signature.returns,
                    delegate.source,
                    binding.source
                )),
                None => errors.push(format!(
                    "{}: delegate '{}' (bound to '{}') not found in the production sources",
                    binding.source, binding.delegate_name, binding.export_name
                )),
            },
            None => errors.push(format!(
                "{}: '{}' is bound in the shim but has no Rust export",
                binding.source, binding.export_name
            )),
        }
    }
    if errors.is_empty() {
        Ok(managed.bindings.len())
    } else {
        Err(errors)
    }
}

fn rust_exports(
    text: &str,
    source: &str,
    bindings: &[Binding],
) -> Result<HashMap<String, Declaration>, String> {
    let file = syn::parse_file(text).map_err(|error| format!("invalid Rust syntax: {error}"))?;
    let mut exports = HashMap::new();
    for item in file.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        let signature = function.sig;
        let name = signature.ident.to_string();
        if !bindings.iter().any(|binding| binding.export_name == name)
            || !signature
                .abi
                .as_ref()
                .and_then(|abi| abi.name.as_ref())
                .is_some_and(|abi| abi.value() == "C")
        {
            continue;
        }
        if !signature.generics.params.is_empty() || signature.variadic.is_some() {
            return Err(format!(
                "{name}: generic or variadic exports are unsupported"
            ));
        }
        let parameters = signature
            .inputs
            .iter()
            .map(|parameter| {
                let syn::FnArg::Typed(parameter) = parameter else {
                    return Err(format!("{name}: self parameters are unsupported"));
                };
                Signature::rust_type(&parameter.ty, false)
                    .map_err(|error| format!("{name}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let returns = match &signature.output {
            syn::ReturnType::Default => "void".to_owned(),
            syn::ReturnType::Type(_, ty) => {
                Signature::rust_type(ty, true).map_err(|error| format!("{name}: {error}"))?
            }
        };
        let declaration = Declaration {
            name: name.clone(),
            signature: Signature {
                parameters,
                returns,
            },
            source: source.to_owned(),
        };
        if exports.insert(name.clone(), declaration).is_some() {
            return Err(format!("{name}: ambiguous Rust export"));
        }
    }
    Ok(exports)
}

impl Signature {
    fn rust_type(ty: &syn::Type, returns: bool) -> Result<String, String> {
        let ident = |ty: &syn::Type| match ty {
            syn::Type::Path(path) if path.qself.is_none() => {
                path.path.get_ident().map(ToString::to_string)
            }
            _ => None,
        };
        let kind = match ty {
            syn::Type::Tuple(tuple) if returns && tuple.elems.is_empty() => Some("void"),
            syn::Type::Ptr(pointer) if !returns => match (
                matches!(pointer.mutability, syn::PointerMutability::Mut(_)),
                ident(&pointer.elem).as_deref(),
            ) {
                (false, Some("c_char")) => Some("string"),
                (true, Some("u8" | "ModifierCredit"))
                | (false, Some("ModifierObservation" | "BlockModifier")) => Some("IntPtr"),
                _ => None,
            },
            _ => match ident(ty).as_deref() {
                Some("i32") => Some("int"),
                Some("u32") if !returns => Some("uint"),
                Some("i64") => Some("long"),
                Some("u64") => Some("ulong"),
                Some("f64") => Some("double"),
                _ => None,
            },
        };
        kind.map(str::to_owned).ok_or_else(|| {
            format!(
                "unsupported Rust {} type '{}'",
                if returns { "return" } else { "parameter" },
                ty.to_token_stream()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = r#"
        pub unsafe extern "C" fn spire_profiler_foo(amount: i32, id: *const c_char, hash: u64) {}
        pub extern "C" fn spire_profiler_bar(amount: i32, started_at: i64, delta: f64) {}
        pub extern "C" fn spire_profiler_unbound(value: Custom) -> Custom { value }
    "#;
    const DECLARATIONS: &str = r#"
        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void NativeFoo(int amount, [MarshalAs(UnmanagedType.LPUTF8Str)] string id, ulong hash);
        private delegate void NativeBar(int amount, long started_at, double delta);
        [return: MarshalAs(UnmanagedType.I4)]
        private delegate Custom Unbound(Custom value);
    "#;
    const BINDINGS: &str = r#"
        _foo = GetExport<NativeFoo>(lib, "spire_profiler_foo");
        _bar = GetExport<NativeBar>(lib, "spire_profiler_bar");
    "#;

    fn managed(declarations: &str, bindings: &str) -> String {
        format!(
            "namespace Fixture; partial class Native {{ {declarations} void Load(IntPtr lib) {{ {bindings} }} }}"
        )
    }

    fn compare(rust: &str, cs: &str) -> Result<usize, Vec<String>> {
        compare_sources(rust, "abi.rs", &[("fixture.cs", cs)])
    }

    fn scalar(rust_type: &str, cs_type: &str) -> Result<usize, Vec<String>> {
        let rust = format!(
            "pub extern \"C\" fn spire_profiler_value(value: i32) {rust_type} {{ todo!() }}"
        );
        compare(
            &rust,
            &managed(
                &format!("private delegate {cs_type} Native(int value);"),
                "GetExport<Native>(lib, \"spire_profiler_value\");",
            ),
        )
    }

    #[test]
    fn production_shapes_resolve_across_sources_in_either_order() {
        let declarations = managed(DECLARATIONS, "");
        let bindings = managed("", BINDINGS);
        for sources in [
            [
                ("Declarations.cs", declarations.as_str()),
                ("Bindings.cs", bindings.as_str()),
            ],
            [
                ("Bindings.cs", bindings.as_str()),
                ("Declarations.cs", declarations.as_str()),
            ],
        ] {
            assert_eq!(compare_sources(RUST, "abi.rs", &sources), Ok(2));
        }
    }

    #[test]
    fn scalar_and_unit_returns_preserve_wire_widths() {
        for (rust, cs) in [
            ("", "void"),
            ("-> ()", "void"),
            ("-> i32", "int"),
            ("-> i64", "long"),
            ("-> u64", "ulong"),
            ("-> f64", "double"),
        ] {
            assert_eq!(scalar(rust, cs), Ok(1));
        }
        for (rust, cs) in [
            ("-> u64", "long"),
            ("-> i32", "double"),
            ("", "ulong"),
            ("-> u64", "void"),
        ] {
            let errors = scalar(rust, cs).expect_err("different return representations fail");
            assert!(errors[0].contains(" != C# "), "{errors:?}");
        }
    }

    #[test]
    fn unsupported_and_malformed_types_fail_closed() {
        for (rust, cs) in [
            ("-> bool", "bool"),
            ("-> Custom", "Custom"),
            ("-> *const c_char", "string"),
            ("-> u64", "bool"),
            ("-> u64", "ulong[]"),
            ("-> u64", "ref ulong"),
            ("->", "void"),
            ("u64", "ulong"),
        ] {
            assert!(scalar(rust, cs).is_err(), "accepted {rust} / {cs}");
        }
        let cs = managed(DECLARATIONS, BINDINGS);
        for rust in [
            RUST.replace("amount: i32", "amount: Custom"),
            RUST.replace("delta: f64", "delta: u32"),
        ] {
            assert!(compare(&rust, &cs).is_err());
        }
        assert!(compare(RUST, &cs.replace("int amount", "Custom amount")).is_err());
        assert!(compare("pub extern \"C\" fn spire_profiler_foo() -> u64", &cs).is_err());
        assert!(compare(RUST, &cs.replace("double delta);", "double delta) extra;")).is_err());
    }

    #[test]
    fn caller_owned_buffers_and_unsigned_sequences_preserve_wire_widths() {
        let rust = r#"pub unsafe extern "C" fn spire_profiler_copy(epoch: u32, buffer: *mut u8, capacity: i32) -> i32 { 0 }"#;
        let cs = managed(
            "private delegate int NativeCopy(uint epoch, IntPtr buffer, int capacity);",
            r#"GetExport<NativeCopy>(lib, "spire_profiler_copy");"#,
        );
        assert_eq!(compare(rust, &cs), Ok(1));
        for wrong in [
            cs.replace("uint epoch", "int epoch"),
            cs.replace("IntPtr buffer", "ulong buffer"),
        ] {
            assert!(compare(rust, &wrong).is_err());
        }
    }

    #[test]
    fn missing_and_ambiguous_declarations_fail_with_source_names() {
        let declarations = managed(DECLARATIONS, "");
        let bindings = managed("", BINDINGS);
        let errors = compare_sources(RUST, "abi.rs", &[("Bindings.cs", &bindings)])
            .expect_err("bindings require declarations");
        assert!(errors[0].contains("Bindings.cs: delegate 'NativeFoo'"));
        let errors = compare_sources(
            RUST,
            "abi.rs",
            &[
                ("Bindings.cs", &bindings),
                ("First.cs", &declarations),
                ("Second.cs", &declarations),
            ],
        )
        .expect_err("ambiguous declarations cannot establish ABI conformance");
        assert!(
            errors[0].contains(
                "Second.cs: NativeFoo: ambiguous bound delegate, also declared in First.cs"
            )
        );
        let cs = managed(
            DECLARATIONS,
            &BINDINGS.replace("spire_profiler_foo", "spire_profiler_missing"),
        );
        assert!(
            compare(RUST, &cs).expect_err("missing exports fail")[0].contains("has no Rust export")
        );
        let duplicate = format!("{RUST} pub extern \"C\" fn spire_profiler_foo() {{}}");
        assert!(
            compare(&duplicate, &managed(DECLARATIONS, BINDINGS))
                .expect_err("duplicate exports fail")[0]
                .contains("ambiguous Rust export")
        );
    }

    #[test]
    fn reordered_parameters_report_a_signature_diff() {
        let cs = managed(
            &DECLARATIONS.replace("int amount, long started_at", "long started_at, int amount"),
            BINDINGS,
        );
        let errors = compare(RUST, &cs).expect_err("parameter order belongs to the wire contract");
        assert_eq!(
            errors,
            [
                "spire_profiler_bar: Rust(int, long, double) -> void [abi.rs] != C# NativeBar(long, int, double) -> void [fixture.cs]; binding in fixture.cs"
            ]
        );
    }

    #[test]
    fn unknown_attributes_and_marshalling_never_compare_equal() {
        for attribute in [
            "[return: MarshalAs(UnmanagedType.I4)]",
            "[UnmanagedFunctionPointer(CallingConvention.StdCall)]",
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl, CharSet = CharSet.Ansi)]",
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl)][UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
        ] {
            let declaration = DECLARATIONS.replace(
                "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]",
                attribute,
            );
            let error = compare(RUST, &managed(&declaration, BINDINGS))
                .expect_err("unknown attributes can change the ABI");
            assert!(
                error[0].contains("unsupported delegate attributes"),
                "{error:?}"
            );
        }
        for replacement in [
            "",
            "[MarshalAs(UnmanagedType.LPStr)]",
            "[MarshalAs(UnmanagedType.LPUTF8Str, SizeConst = 8)]",
        ] {
            let declarations =
                DECLARATIONS.replace("[MarshalAs(UnmanagedType.LPUTF8Str)]", replacement);
            assert!(compare(RUST, &managed(&declarations, BINDINGS)).is_err());
        }
        for parameter in [
            "ref int amount",
            "[MarshalAs(UnmanagedType.I8)] int amount",
            "Custom amount",
        ] {
            assert!(
                compare(
                    RUST,
                    &managed(&DECLARATIONS.replace("int amount", parameter), BINDINGS)
                )
                .is_err()
            );
        }
    }

    #[test]
    fn comments_whitespace_and_regions_cannot_hide_return_attributes() {
        for newline in ["\n", "\r", "\r\n"] {
            let declarations = DECLARATIONS.replace(
                "private delegate void NativeFoo",
                "/* nested-looking { ( ] */\nprivate delegate void NativeFoo",
            );
            let bindings = BINDINGS.replace(
                "GetExport<NativeFoo>(lib,",
                "GetExport < NativeFoo > ( lib ,",
            );
            assert_eq!(
                compare(
                    RUST,
                    &managed(&declarations, &bindings).replace('\n', newline)
                ),
                Ok(2)
            );
            let declarations = DECLARATIONS.replace("private delegate void NativeFoo", "[return: MarshalAs(UnmanagedType.I4)]\n#region native result } // comment\nprivate delegate void NativeFoo")
                .replace("ulong hash);", "ulong hash);\n#endregion");
            assert!(
                compare(
                    RUST,
                    &managed(&declarations, BINDINGS).replace('\n', newline)
                )
                .is_err()
            );
        }
    }

    #[test]
    fn fake_bindings_and_exports_in_comments_and_strings_are_ignored() {
        let rust = format!(
            r###"{RUST}
            // extern "C" fn spire_profiler_foo(amount: u64) {{}}
            const TEXT: &str = r##"extern "C" fn spire_profiler_foo(amount: u64) {{}}"##;
        "###
        );
        let cs = managed(
            DECLARATIONS,
            &format!(
                r#"{BINDINGS}
            // GetExport<Fake>(broken);
            var example = "GetExport<Fake>(broken)";
        "#
            ),
        );
        assert_eq!(compare(&rust, &cs), Ok(2));
    }

    #[test]
    fn malformed_bindings_and_empty_inputs_fail_alongside_valid_code() {
        for bad in [
            r#"GetExport<NativeBar>(other, "spire_profiler_bar");"#,
            r#"GetExport<NativeBar>(lib, "_profiler_bar");"#,
            r#"GetExport<NativeBar>(lib, "spire_profiler_");"#,
            r#"GetExport<NativeBar>(lib, name);"#,
            r#"GetExport<NativeBar>(IntPtr lib, "spire_profiler_bar");"#,
            "\n#if UNKNOWN\nGetExport<NativeBar>(lib, \"spire_profiler_bar\");\n#endif\n",
        ] {
            let errors = compare_sources(
                RUST,
                "abi.rs",
                &[
                    ("Valid.cs", &managed(DECLARATIONS, BINDINGS)),
                    ("Broken.cs", &managed("", bad)),
                ],
            )
            .expect_err("one invalid binding invalidates the check");
            assert!(errors[0].contains("Broken.cs"), "{errors:?}");
        }
        assert_eq!(
            compare(RUST, ""),
            Err(vec![
                "no GetExport bindings were parsed from the production sources".to_owned()
            ])
        );
    }
}
