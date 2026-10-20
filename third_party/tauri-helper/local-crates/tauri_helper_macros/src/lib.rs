use proc_macro::TokenStream;
use quote::quote;
use std::path::Path;
use std::{
    collections::BTreeSet,
    env,
    fs::{self},
};
#[cfg(feature = "tracing")]
use syn::{Data, DeriveInput, Fields};
use syn::{LitBool, parse_macro_input};
use tauri_helper_core::{find_workspace_dir, get_workspace_pkg_name};

#[cfg(feature = "tracing")]
fn is_string_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == "String";
        }
    }
    false
}

/// Derive macro for adding logging capabilities to enum variants.
///
/// This macro automatically implements `From<T>` for specified types,
/// emitting `tracing` logs whenever a conversion occurs.
///
/// # Example
///
/// ```rust
/// use tracing::error;
///
/// #[derive(WithLogging)]
/// enum Error {
///     #[logging_from(String)]
///     StringError(String),
///
///     #[logging_from(i32)]
///     IntError(i32),
///
///     StructError { code: i32, message: String },
/// }
/// ```
#[cfg(feature = "tracing")]
#[proc_macro_derive(WithLogging, attributes(logging_from, no_from_string))]
pub fn derive_with_logging(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let mut from_impls = vec![];

    if let Data::Enum(ref data_enum) = input.data {
        for variant in &data_enum.variants {
            let variant_name = &variant.ident;

            match &variant.fields {
                // Single unnamed field (e.g., SomeError(String))
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    if let Some(attr) = variant
                        .attrs
                        .iter()
                        .find(|a| a.path().is_ident("logging_from"))
                    {
                        let convert_type = attr.parse_args::<syn::Type>().unwrap();
                        let field_type = &fields.unnamed.first().unwrap().ty;

                        let conversion = if is_string_type(field_type) {
                            quote! { value.to_string() }
                        } else {
                            quote! { value.into() }
                        };

                        let from_impl = quote! {
                            impl From<#convert_type> for #name {
                                fn from(value: #convert_type) -> Self {
                                    let converted_value: #field_type = #conversion;
                                    tracing::error!(
                                        "Error occurred: {} - {}",
                                        stringify!(#variant_name),
                                        converted_value
                                    );
                                    #name::#variant_name(converted_value)
                                }
                            }
                        };
                        from_impls.push(from_impl);
                    }
                }
                // Multiple unnamed fields (e.g., SomeError(String, i32))
                Fields::Unnamed(fields) => {
                    let field_types: Vec<_> = fields.unnamed.iter().map(|f| &f.ty).collect();
                    let field_names: Vec<_> = (0..field_types.len())
                        .map(|i| {
                            syn::Ident::new(&format!("field{}", i), proc_macro2::Span::call_site())
                        })
                        .collect();

                    let from_impl = quote! {
                        impl From<(#(#field_types),*)> for #name {
                            fn from(value: (#(#field_types),*)) -> Self {
                                let (#(#field_names),*) = value;
                                let err_str = format!("{:?}", (#(#field_names),*));
                                tracing::error!("Error occurred: {} - {}", stringify!(#variant_name), err_str);
                                #name::#variant_name(#(#field_names),*)
                            }
                        }
                    };
                    from_impls.push(from_impl);
                }

                // Struct-like fields (e.g., SomeError { message: String, code: i32 })
                Fields::Named(fields) => {
                    let field_names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
                    let field_types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();

                    let from_impl = quote! {
                        impl From<(#(#field_types),*)> for #name {
                            fn from(value: (#(#field_types),*)) -> Self {
                                let (#(#field_names),*) = value;
                                let err_str = format!("{:?}", (#(&#field_names),*));
                                tracing::error!(
                                    "Error occurred: {} - {}",
                                    stringify!(#variant_name),
                                    err_str
                                );
                                #name::#variant_name { #(#field_names),* }
                            }
                        }
                    };
                    from_impls.push(from_impl);
                }

                _ => {}
            }
        }
    }

    TokenStream::from(quote! { #(#from_impls)* })
}

fn is_specta_command_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with("_specta"))
}

/// Reads command paths from the generated txt files.
///
/// `specta_only = true`  → reads `*_specta.txt` (for `specta_collect_commands!`)
/// `specta_only = false` → reads non-specta `*.txt`  (for `tauri_collect_commands!`)
///
/// Returns paths sorted by function name (last `::` segment) so the resulting
/// TypeScript bindings come out in alphabetical order by command name.
fn collect_commands(specta_only: bool) -> Vec<String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let workspace_root = find_workspace_dir(Path::new(&manifest_dir));
    let commands_dir = workspace_root.join("target").join("tauri_commands_list");

    let calling_crate = get_workspace_pkg_name().replace('-', "_");
    let crate_prefix = format!("{calling_crate}::");

    let mut commands = BTreeSet::new();

    if let Ok(entries) = fs::read_dir(&commands_dir) {
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();

        for path in paths {
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            if specta_only != is_specta_command_file(&path) {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    // Strip the calling crate's prefix so the generated Rust path is
                    // relative (e.g. `audio_exclusions::read_x` instead of
                    // `screenpipe_app_tauri::audio_exclusions::read_x`).
                    let fn_path = trimmed
                        .strip_prefix(&crate_prefix)
                        .unwrap_or(trimmed)
                        .to_string();

                    if fn_path
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
                    {
                        commands.insert(fn_path);
                    } else {
                        panic!(
                            "tauri-helper: invalid command path `{fn_path}` in {}",
                            path.display()
                        );
                    }
                }
            }
        }
    } else {
        eprintln!(
            "tauri-helper warning: commands directory not found at {} — \
             did `generate_command_file` run in build.rs?",
            commands_dir.display()
        );
    }

    // BTreeSet gives lexicographic order by full path; re-sort by function name
    // (last `::` segment) so the TypeScript output is alphabetical by command name.
    let mut result: Vec<String> = commands.into_iter().collect();
    result.sort_by(|a, b| {
        let a_fn = a.rsplit("::").next().unwrap_or(a.as_str());
        let b_fn = b.rsplit("::").next().unwrap_or(b.as_str());
        a_fn.cmp(b_fn)
    });
    result
}

fn parse_command_path(fn_name: &str) -> Result<syn::Path, syn::Error> {
    syn::parse_str::<syn::Path>(fn_name).map_err(|e| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("tauri-helper: invalid command path `{fn_name}` in generated txt file: {e}"),
        )
    })
}

/// Generates the `tauri_specta::collect_commands![]` invocation for all commands
/// that carry both `#[tauri::command]` and `#[specta::specta]`.
#[proc_macro]
pub fn specta_collect_commands(_item: TokenStream) -> TokenStream {
    let commands = collect_commands(true);

    if commands.is_empty() {
        eprintln!(
            "tauri-helper warning: no specta commands collected — \
             ensure commands have both `#[tauri::command]` and `#[specta::specta]`"
        );
        return quote! { tauri_specta::collect_commands![] }.into();
    }

    let mut paths = Vec::new();
    for fn_name in &commands {
        match parse_command_path(fn_name) {
            Ok(path) => paths.push(quote!(#path)),
            Err(e) => return e.into_compile_error().into(),
        }
    }

    quote! { tauri_specta::collect_commands![ #(#paths),* ] }.into()
}

/// Generates the `tauri::generate_handler![]` invocation for all collected
/// `#[tauri::command]` functions.
#[proc_macro]
pub fn tauri_collect_commands(_item: TokenStream) -> TokenStream {
    let commands = collect_commands(false);

    if commands.is_empty() {
        eprintln!(
            "tauri-helper warning: no commands collected — \
             ensure functions are annotated with `#[tauri::command]`"
        );
        return quote! { tauri::generate_handler![] }.into();
    }

    let mut paths = Vec::new();
    for fn_name in &commands {
        match parse_command_path(fn_name) {
            Ok(path) => paths.push(path),
            Err(e) => return e.into_compile_error().into(),
        }
    }

    quote! { tauri::generate_handler![ #(#paths),* ] }.into()
}

/// Expands to a `[&str; N]` array of all collected command paths.
///
/// Pass `true` to bind the array to a local `arr` variable (useful in tests):
/// `array_collect_commands!(true)` → `{ let arr = [...]; arr }`.
#[proc_macro]
pub fn array_collect_commands(item: TokenStream) -> TokenStream {
    let print_arg = parse_macro_input!(item as Option<LitBool>);
    let should_print = print_arg.map(|lit| lit.value()).unwrap_or(false);

    let commands = collect_commands(false);

    if commands.is_empty() {
        return quote! { [] as [&str; 0] }.into();
    }

    let literals: Vec<proc_macro2::Literal> = commands
        .iter()
        .map(|fn_name| proc_macro2::Literal::string(fn_name))
        .collect();

    if should_print {
        quote! { { let arr = [ #(#literals),* ]; arr } }
    } else {
        quote! { [ #(#literals),* ] }
    }
    .into()
}
