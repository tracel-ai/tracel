mod name_value;

use name_value::get_name_value;
use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;

use strum::Display;
use syn::{Attribute, Data, DeriveInput, Field, Fields, Lit, MetaNameValue};
use syn::{Error, ItemFn, Meta, Path, parse_macro_input, punctuated::Punctuated, spanned::Spanned};

#[derive(Eq, Hash, PartialEq, Display)]
#[strum(serialize_all = "PascalCase")]
pub(crate) enum ProcedureType {
    Training,
    Inference,
}

impl TryFrom<Path> for ProcedureType {
    type Error = Error;

    fn try_from(path: Path) -> Result<Self, Self::Error> {
        match path.get_ident() {
            Some(ident) => match ident.to_string().as_str() {
                "training" => Ok(Self::Training),
                "inference" => Ok(Self::Inference),
                _ => Err(Error::new_spanned(
                    path,
                    "Expected `training` or `inference`",
                )),
            },
            None => Err(Error::new_spanned(
                path,
                "Expected `training` or `inference`",
            )),
        }
    }
}

fn compile_errors(errors: Vec<Error>) -> proc_macro2::TokenStream {
    errors
        .into_iter()
        .map(|err| err.to_compile_error())
        .collect()
}

pub(crate) fn generate_flag_register_stream(
    item: &ItemFn,
    builder_fn_ident: &Ident,
    procedure_type: &ProcedureType,
    routine_name: &syn::LitStr,
) -> proc_macro2::TokenStream {
    let fn_name = &item.sig.ident;
    let builder_fn_name = &builder_fn_ident;
    let serialized_fn_item = syn_serde::json::to_string(item);
    let serialized_lit_arr = syn::LitByteStr::new(serialized_fn_item.as_bytes(), item.span());
    let proc_type_str = Ident::new(
        &procedure_type.to_string().to_lowercase(),
        proc_macro2::Span::call_site(),
    );
    quote! {
        burn_central::cli::register_functions!(
            burn_central::cli::tools::functions_registry::FunctionMetadata,
            burn_central::cli::tools::functions_registry::FunctionMetadata::new(
                module_path!(),
                stringify!(#fn_name),
                stringify!(#builder_fn_name),
                #routine_name,
                stringify!(#proc_type_str),
                #serialized_lit_arr));
    }
}

fn get_string_arg(
    args: &Punctuated<Meta, syn::Token![,]>,
    arg_name: &str,
    errors: &mut Vec<Error>,
) -> Option<syn::LitStr> {
    args.iter()
        .find(|meta| meta.path().is_ident(arg_name))
        .and_then(|meta| match meta.require_name_value() {
            Ok(value) => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(name),
                    ..
                }) => Some(name.clone()),
                _ => {
                    errors.push(Error::new(
                        value.value.span(),
                        format!("Expected a string literal for the `{arg_name}` argument."),
                    ));
                    None
                }
            },
            Err(err) => {
                errors.push(err);
                None
            }
        })
}

fn validate_registered_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Registered name cannot be empty.".to_string());
    }
    if name.contains(' ') {
        return Err("Registered name cannot contain spaces.".to_string());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(
            "Registered name can only contain alphanumeric characters and underscores.".to_string(),
        );
    }
    Ok(())
}

#[proc_macro_attribute]
pub fn register(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = Vec::<Error>::new();
    let args = parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as ItemFn);
    let fn_name = &item.sig.ident;

    if args.is_empty() {
        errors.push(Error::new(
            args.span(),
            "Expected one argument for the #[register] attribute. Please provide the procedure type (training or inference) as the first argument.",
        ));
    }

    // Determine the proc type (training or inference)
    let procedure_type = match ProcedureType::try_from(
        args.first()
            .expect("Should be able to get first arg.")
            .path()
            .clone(),
    ) {
        Ok(procedure_type) => procedure_type,
        Err(err) => {
            return err.into_compile_error().into();
        }
    };

    if procedure_type == ProcedureType::Inference {
        errors.push(Error::new_spanned(
            args.first().unwrap().path(),
            "Inference procedures are not supported yet. Please use training procedures.",
        ));
    }

    let maybe_registered_name = get_string_arg(&args, "name", &mut errors);

    if let Some(name) = &maybe_registered_name {
        if let Err(err) = validate_registered_name(&name.value()) {
            errors.push(Error::new_spanned(
                name,
                format!("Invalid registered name: {err}"),
            ));
        }
    }

    let builder_fn_name = syn::Ident::new(
        &format!("__{fn_name}_builder"),
        proc_macro2::Span::call_site(),
    );

    let registered_name_str = {
        let name = maybe_registered_name
            .map(|name| name.value())
            .unwrap_or_else(|| fn_name.to_string());

        syn::LitStr::new(&name, fn_name.span())
    };

    let builder_item = match procedure_type {
        ProcedureType::Training => {
            quote! {
                #[doc(hidden)]
                pub fn #builder_fn_name<B: burn::tensor::backend::AutodiffBackend>(
                    exec: &mut burn_central::runtime::ExecutorBuilder<B>,
                ) {
                    exec.train(#registered_name_str, #fn_name::<B>);
                }
            }
        }
        ProcedureType::Inference => {
            quote! {}
        }
    };

    let flag_register = if cfg!(feature = "build-cli") {
        generate_flag_register_stream(
            &item,
            &builder_fn_name,
            &procedure_type,
            &registered_name_str,
        )
    } else {
        quote! {}
    };

    let code = quote! {
        #[allow(dead_code)]
        #item

        #flag_register
        #builder_item
    };

    // If there are any errors, combine them and return
    if !errors.is_empty() {
        compile_errors(errors).into()
    } else {
        code.into()
    }
}

#[proc_macro_attribute]
pub fn burn_central_main(args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let args: Punctuated<Meta, syn::token::Comma> =
        parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);

    let module_path = args
        .first()
        .expect("Should be able to get first arg.")
        .path()
        .clone();
    let api_endpoint: Option<String> = get_name_value(&args, "api_endpoint");

    let mut config_block = quote! {
        let mut config = burn_central::cli::config::Config::default();
    };
    if let Some(api_endpoint) = api_endpoint {
        config_block.extend(quote! {
            config.api_endpoint = #api_endpoint.to_string();
        });
    }

    let item_sig = &item.sig;
    let item_block = &item.block;

    // cause an error if the function has a body
    if !item_block.stmts.is_empty() {
        return Error::new(
            item_block.span(),
            "The cli main function should not have a body",
        )
        .to_compile_error()
        .into();
    }

    let mod_tokens = syn::Ident::new(
        &format!("__{}", uuid::Uuid::new_v4().simple()),
        proc_macro2::Span::call_site(),
    );

    let main_call = if option_env!("COMPUTE_PROVIDER_RUNTIME").is_some() {
        quote! {
            burn_central::cli::compute_provider::compute_provider_main(config);
        }
    } else {
        quote! {
            burn_central::cli::cli::cli_main(config);
        }
    };

    let item = quote! {
        mod #mod_tokens {
            #[allow(unused_imports)]
            pub use #module_path;
        }

        #item_sig {
            #config_block

            #main_call
        }
    };

    let code = quote! {
        #item
    };

    code.into()
}

#[derive(Debug, Clone)]
struct BundleFieldConfig {
    file_path: String,
    format: Option<String>,
    with_codec: Option<String>,
    optional: bool,
    settings_type: Option<syn::ExprPath>,
}

impl BundleFieldConfig {
    fn parse_from_attrs(attrs: &[Attribute]) -> Result<Option<Self>, Error> {
        for attr in attrs {
            if attr.path().is_ident("bundle") {
                return Self::parse_bundle_attr(attr).map(Some);
            }
        }
        Ok(None)
    }

    fn parse_bundle_attr(attr: &Attribute) -> Result<Self, Error> {
        let mut file_path = None;
        let mut format = None;
        let mut with_codec = None;
        let mut optional = false;
        let mut settings_type = None;

        match &attr.meta {
            Meta::List(meta_list) => {
                let parsed_args = meta_list
                    .parse_args_with(
                        syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                    )
                    .map_err(|e| {
                        Error::new_spanned(
                            meta_list,
                            format!("Failed to parse bundle attributes: {}", e),
                        )
                    })?;

                for meta in parsed_args.iter() {
                    match meta {
                        Meta::NameValue(MetaNameValue { path, value, .. }) => {
                            if path.is_ident("file") {
                                if let syn::Expr::Lit(syn::ExprLit {
                                    lit: Lit::Str(s), ..
                                }) = value
                                {
                                    file_path = Some(s.value());
                                } else {
                                    return Err(Error::new_spanned(
                                        value,
                                        "Expected string literal for file",
                                    ));
                                }
                            } else if path.is_ident("format") {
                                if let syn::Expr::Lit(syn::ExprLit {
                                    lit: Lit::Str(s), ..
                                }) = value
                                {
                                    format = Some(s.value());
                                } else {
                                    return Err(Error::new_spanned(
                                        value,
                                        "Expected string literal for format",
                                    ));
                                }
                            } else if path.is_ident("with") {
                                if let syn::Expr::Lit(syn::ExprLit {
                                    lit: Lit::Str(s), ..
                                }) = value
                                {
                                    with_codec = Some(s.value());
                                } else {
                                    return Err(Error::new_spanned(
                                        value,
                                        "Expected string literal for with",
                                    ));
                                }
                            } else if path.is_ident("settings") {
                                if let syn::Expr::Path(expr_path) = value {
                                    settings_type = Some(expr_path.clone());
                                } else {
                                    return Err(Error::new_spanned(
                                        value,
                                        "Expected a type path for settings (e.g., settings = MyType)",
                                    ));
                                }
                            }
                        }
                        Meta::Path(path) => {
                            if path.is_ident("optional") {
                                optional = true;
                            }
                        }
                        _ => return Err(Error::new_spanned(meta, "Unexpected attribute format")),
                    }
                }
            }
            _ => return Err(Error::new_spanned(attr, "Expected #[bundle(...)] format")),
        }

        let file_path = file_path
            .ok_or_else(|| Error::new_spanned(attr, "Missing required 'file' attribute"))?;

        Ok(BundleFieldConfig {
            file_path,
            format,
            with_codec,
            optional,
            settings_type,
        })
    }
}

fn generate_encode_field(
    field: &Field,
    config: &BundleFieldConfig,
) -> Result<proc_macro2::TokenStream, Error> {
    let field_name = field.ident.as_ref().unwrap();
    let file_path = &config.file_path;

    if config.optional {
        if let Some(codec) = &config.with_codec {
            let encode_fn = syn::Ident::new(&format!("{}_encode", codec), field.span());
            if config.settings_type.is_some() {
                let settings_field =
                    syn::Ident::new(&format!("{}_settings", field_name), field.span());
                Ok(quote! {
                    if let Some(value) = self.#field_name {
                        let bytes = #encode_fn(value, settings.#settings_field.as_ref())
                            .map_err(|e| format!("Failed to encode {}: {}", #file_path, e))?;
                        sink.put_bytes(#file_path, &bytes)
                            .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                    }
                })
            } else {
                Ok(quote! {
                    if let Some(value) = self.#field_name {
                        let bytes = #encode_fn(value)
                            .map_err(|e| format!("Failed to encode {}: {}", #file_path, e))?;
                        sink.put_bytes(#file_path, &bytes)
                            .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                    }
                })
            }
        } else {
            match config.format.as_deref() {
                Some("json") => Ok(quote! {
                    if let Some(ref value) = self.#field_name {
                        let bytes = serde_json::to_vec(value)
                            .map_err(|e| format!("Failed to serialize {} to JSON: {}", #file_path, e))?;
                        sink.put_bytes(#file_path, &bytes)
                            .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                    }
                }),
                Some("raw-utf8") => Ok(quote! {
                    if let Some(ref value) = self.#field_name {
                        let bytes = value.as_bytes();
                        sink.put_bytes(#file_path, bytes)
                            .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                    }
                }),
                Some(format) => Err(Error::new_spanned(
                    field,
                    format!("Unsupported format: {}", format),
                )),
                None => Err(Error::new_spanned(
                    field,
                    "Missing format or with attribute for field",
                )),
            }
        }
    } else {
        if let Some(codec) = &config.with_codec {
            let encode_fn = syn::Ident::new(&format!("{}_encode", codec), field.span());
            if config.settings_type.is_some() {
                let settings_field =
                    syn::Ident::new(&format!("{}_settings", field_name), field.span());
                Ok(quote! {
                    let bytes = #encode_fn(self.#field_name, settings.#settings_field.as_ref())
                        .map_err(|e| format!("Failed to encode {}: {}", #file_path, e))?;
                    sink.put_bytes(#file_path, &bytes)
                        .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                })
            } else {
                Ok(quote! {
                    let bytes = #encode_fn(self.#field_name)
                        .map_err(|e| format!("Failed to encode {}: {}", #file_path, e))?;
                    sink.put_bytes(#file_path, &bytes)
                        .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                })
            }
        } else {
            match config.format.as_deref() {
                Some("json") => Ok(quote! {
                    let bytes = serde_json::to_vec(&self.#field_name)
                        .map_err(|e| format!("Failed to serialize {} to JSON: {}", #file_path, e))?;
                    sink.put_bytes(#file_path, &bytes)
                        .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                }),
                Some("raw-utf8") => Ok(quote! {
                    let bytes = self.#field_name.as_bytes();
                    sink.put_bytes(#file_path, bytes)
                        .map_err(|e| format!("Failed to write {}: {}", #file_path, e))?;
                }),
                Some(format) => Err(Error::new_spanned(
                    field,
                    format!("Unsupported format: {}", format),
                )),
                None => Err(Error::new_spanned(
                    field,
                    "Missing format or with attribute for field",
                )),
            }
        }
    }
}

fn generate_decode_field(
    field: &Field,
    config: &BundleFieldConfig,
) -> Result<proc_macro2::TokenStream, Error> {
    let field_name = field.ident.as_ref().unwrap();
    let file_path = &config.file_path;

    if config.optional {
        if let Some(codec) = &config.with_codec {
            let decode_fn = syn::Ident::new(&format!("{}_decode", codec), field.span());
            if config.settings_type.is_some() {
                let settings_field =
                    syn::Ident::new(&format!("{}_settings", field_name), field.span());
                Ok(quote! {
                    #field_name: {
                        match source.open(#file_path) {
                            Ok(mut reader) => {
                                let mut bytes = Vec::new();
                                reader.read_to_end(&mut bytes)
                                    .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                                Some(#decode_fn(&bytes, settings.#settings_field.as_ref())
                                    .map_err(|e| format!("Failed to decode {}: {}", #file_path, e))?)
                            }
                            Err(_) => None,
                        }
                    },
                })
            } else {
                Ok(quote! {
                    #field_name: {
                        match source.open(#file_path) {
                            Ok(mut reader) => {
                                let mut bytes = Vec::new();
                                reader.read_to_end(&mut bytes)
                                    .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                                Some(#decode_fn(&bytes)
                                    .map_err(|e| format!("Failed to decode {}: {}", #file_path, e))?)
                            }
                            Err(_) => None,
                        }
                    },
                })
            }
        } else {
            match config.format.as_deref() {
                Some("json") => Ok(quote! {
                    #field_name: {
                        match source.open(#file_path) {
                            Ok(reader) => Some(serde_json::from_reader(reader)
                                .map_err(|e| format!("Failed to deserialize {} from JSON: {}", #file_path, e))?),
                            Err(_) => None,
                        }
                    },
                }),
                Some("raw-utf8") => Ok(quote! {
                    #field_name: {
                        match source.open(#file_path) {
                            Ok(mut reader) => {
                                let mut bytes = Vec::new();
                                reader.read_to_end(&mut bytes)
                                    .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                                Some(String::from_utf8(bytes)
                                    .map_err(|e| format!("Failed to parse {} as UTF-8: {}", #file_path, e))?)
                            }
                            Err(_) => None,
                        }
                    },
                }),
                Some(format) => Err(Error::new_spanned(
                    field,
                    format!("Unsupported format: {}", format),
                )),
                None => Err(Error::new_spanned(
                    field,
                    "Missing format or with attribute for field",
                )),
            }
        }
    } else {
        if let Some(codec) = &config.with_codec {
            let decode_fn = syn::Ident::new(&format!("{}_decode", codec), field.span());
            if config.settings_type.is_some() {
                let settings_field =
                    syn::Ident::new(&format!("{}_settings", field_name), field.span());
                Ok(quote! {
                    #field_name: {
                        let mut reader = source.open(#file_path)
                            .map_err(|e| format!("Failed to open {}: {}", #file_path, e))?;
                        let mut bytes = Vec::new();
                        reader.read_to_end(&mut bytes)
                            .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                        #decode_fn(&bytes, settings.#settings_field.as_ref())
                            .map_err(|e| format!("Failed to decode {}: {}", #file_path, e))?
                    },
                })
            } else {
                Ok(quote! {
                    #field_name: {
                        let mut reader = source.open(#file_path)
                            .map_err(|e| format!("Failed to open {}: {}", #file_path, e))?;
                        let mut bytes = Vec::new();
                        reader.read_to_end(&mut bytes)
                            .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                        #decode_fn(&bytes)
                            .map_err(|e| format!("Failed to decode {}: {}", #file_path, e))?
                    },
                })
            }
        } else {
            match config.format.as_deref() {
                Some("json") => Ok(quote! {
                    #field_name: {
                        let reader = source.open(#file_path)
                            .map_err(|e| format!("Failed to open {}: {}", #file_path, e))?;
                        serde_json::from_reader(reader)
                            .map_err(|e| format!("Failed to deserialize {} from JSON: {}", #file_path, e))?
                    },
                }),
                Some("raw-utf8") => Ok(quote! {
                    #field_name: {
                        let mut reader = source.open(#file_path)
                            .map_err(|e| format!("Failed to open {}: {}", #file_path, e))?;
                        let mut bytes = Vec::new();
                        reader.read_to_end(&mut bytes)
                            .map_err(|e| format!("Failed to read {}: {}", #file_path, e))?;
                        String::from_utf8(bytes)
                            .map_err(|e| format!("Failed to parse {} as UTF-8: {}", #file_path, e))?
                    },
                }),
                Some(format) => Err(Error::new_spanned(
                    field,
                    format!("Unsupported format: {}", format),
                )),
                None => Err(Error::new_spanned(
                    field,
                    "Missing format or with attribute for field",
                )),
            }
        }
    }
}

#[proc_macro_derive(Bundle, attributes(bundle))]
pub fn derive_bundle(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Error::new_spanned(
                    &input,
                    "Bundle derive only supports structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return Error::new_spanned(&input, "Bundle derive only supports structs")
                .to_compile_error()
                .into();
        }
    };

    let mut encode_statements = Vec::new();
    let mut decode_statements = Vec::new();
    let mut settings_fields = Vec::new();

    for field in fields {
        let config = match BundleFieldConfig::parse_from_attrs(&field.attrs) {
            Ok(Some(config)) => config,
            Ok(None) => {
                return Error::new_spanned(field, "Field missing #[bundle(...)] attribute")
                    .to_compile_error()
                    .into();
            }
            Err(e) => return e.to_compile_error().into(),
        };

        // Generate settings field if needed
        if let Some(settings_type) = &config.settings_type {
            let field_name = field.ident.as_ref().unwrap();
            let settings_field_name =
                syn::Ident::new(&format!("{}_settings", field_name), field.span());
            let settings_type_ident = &settings_type.path;

            settings_fields.push(quote! {
                pub #settings_field_name: Option<#settings_type_ident>
            });
        }

        let encode_stmt = match generate_encode_field(field, &config) {
            Ok(stmt) => stmt,
            Err(e) => return e.to_compile_error().into(),
        };

        let decode_stmt = match generate_decode_field(field, &config) {
            Ok(stmt) => stmt,
            Err(e) => return e.to_compile_error().into(),
        };

        encode_statements.push(encode_stmt);
        decode_statements.push(decode_stmt);
    }

    // Generate settings struct name
    let settings_name = syn::Ident::new(&format!("{}Settings", name), name.span());

    // Determine if we need a custom settings struct
    let (settings_type, settings_struct) = if settings_fields.is_empty() {
        (quote! { () }, quote! {})
    } else {
        (
            quote! { #settings_name },
            quote! {
                #[derive(serde::Serialize, serde::Deserialize, Default)]
                pub struct #settings_name {
                    #(#settings_fields,)*
                }
            },
        )
    };

    let settings_param = if settings_fields.is_empty() {
        quote! { _settings }
    } else {
        quote! { settings }
    };

    let expanded = quote! {
        #settings_struct

        const _: () = {
            #[allow(unused_extern_crates, clippy::useless_attribute)]
            extern crate burn_central as _burn_central;

            #[automatically_derived]
            impl #impl_generics _burn_central::bundle::BundleEncode for #name #ty_generics #where_clause {
                type Settings = #settings_type;
                type Error = String;

                fn encode<O: _burn_central::bundle::BundleSink>(
                    self,
                    sink: &mut O,
                    #settings_param: &Self::Settings,
                ) -> Result<(), Self::Error> {
                    use std::io::Read;

                    #(#encode_statements)*

                    Ok(())
                }
            }
            #[automatically_derived]
            impl #impl_generics _burn_central::bundle::BundleDecode for #name #ty_generics #where_clause {
                type Settings = #settings_type;
                type Error = String;

                fn decode<I: _burn_central::bundle::BundleSource>(
                    source: &I,
                    #settings_param: &Self::Settings,
                ) -> Result<Self, Self::Error> {
                    use std::io::Read;

                    Ok(Self {
                        #(#decode_statements)*
                    })
                }
            }
        };
    };

    TokenStream::from(expanded)
}

#[cfg(test)]
mod test_bundle_settings {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_bundle_field_config_with_settings() {
        let attr: syn::Attribute = parse_quote! {
            #[bundle(file = "weights.bin", with = "weights_codec", settings = "WeightsSettings")]
        };

        let config = BundleFieldConfig::parse_bundle_attr(&attr).unwrap();

        assert_eq!(config.file_path, "weights.bin");
        assert_eq!(config.with_codec, Some("weights_codec".to_string()));
        assert_eq!(config.settings_type, Some(parse_quote! { WeightsSettings }));
        assert!(!config.optional);
    }

    #[test]
    fn test_bundle_field_config_without_settings() {
        let attr: syn::Attribute = parse_quote! {
            #[bundle(file = "config.json", format = "json")]
        };

        let config = BundleFieldConfig::parse_bundle_attr(&attr).unwrap();

        assert_eq!(config.file_path, "config.json");
        assert_eq!(config.format, Some("json".to_string()));
        assert_eq!(config.settings_type, None);
        assert!(!config.optional);
    }
}
