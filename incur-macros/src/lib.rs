//! Derive macros for `incur`.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, LitStr, parse_macro_input};

/// Derives serialization and JSON Schema reflection for an Incur handler output.
#[proc_macro_derive(IncurOutput, attributes(serde, schemars))]
pub fn incur_output(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_incur_output(input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Runs an async CLI entrypoint on Incur's re-exported Tokio runtime.
#[proc_macro_attribute]
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = TokenStream2::from(args);
    let item = TokenStream2::from(item);
    let (incur, incur_name) = incur_path();
    let tokio_path = LitStr::new(&format!("{incur_name}::tokio"), Span::call_site());
    let runtime_args = if args.is_empty() {
        quote!(crate = #tokio_path)
    } else {
        quote!(#args, crate = #tokio_path)
    };

    quote! {
        #[#incur::tokio::main(#runtime_args)]
        #item
    }
    .into()
}

fn expand_incur_output(input: DeriveInput) -> syn::Result<TokenStream2> {
    if matches!(input.data, Data::Union(_)) {
        return Err(syn::Error::new_spanned(
            input,
            "IncurOutput can only be derived for structs and enums",
        ));
    }

    let (incur, incur_name) = incur_path();
    let serde_path = LitStr::new(&format!("{incur_name}::serde"), Span::call_site());
    let schemars_path = LitStr::new(&format!("{incur_name}::schemars"), Span::call_site());
    let ident = &input.ident;
    let helper_ident = format_ident!("__IncurOutputFor{ident}");
    let remote_path = LitStr::new(&ident.to_string(), ident.span());

    let mut helper = input.clone();
    helper.ident = helper_ident.clone();
    helper.vis = syn::Visibility::Inherited;
    helper.attrs.retain(|attribute| !attribute.path().is_ident("derive"));
    helper.attrs.insert(
        0,
        syn::parse_quote!(#[derive(#incur::serde::Serialize, #incur::schemars::JsonSchema)]),
    );
    helper.attrs.insert(1, syn::parse_quote!(#[serde(remote = #remote_path, crate = #serde_path)]));
    helper.attrs.insert(2, syn::parse_quote!(#[schemars(crate = #schemars_path)]));

    let (_, type_generics, _) = input.generics.split_for_impl();
    let mut serialize_generics = input.generics.clone();
    for parameter in serialize_generics.type_params_mut() {
        parameter.bounds.push(syn::parse_quote!(#incur::serde::Serialize));
    }
    let (serialize_impl_generics, _, serialize_where_clause) = serialize_generics.split_for_impl();
    let mut schema_generics = input.generics.clone();
    for parameter in schema_generics.type_params_mut() {
        parameter.bounds.push(syn::parse_quote!(#incur::schemars::JsonSchema));
    }
    let (schema_impl_generics, _, schema_where_clause) = schema_generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            #helper

            #[automatically_derived]
            impl #serialize_impl_generics #incur::serde::Serialize for #ident #type_generics
                #serialize_where_clause
            {
                fn serialize<__S>(&self, serializer: __S) -> ::core::result::Result<__S::Ok, __S::Error>
                where
                    __S: #incur::serde::Serializer,
                {
                    #helper_ident::serialize(self, serializer)
                }
            }

            #[automatically_derived]
            impl #schema_impl_generics #incur::schemars::JsonSchema for #ident #type_generics
                #schema_where_clause
            {
                fn inline_schema() -> bool {
                    <#helper_ident #type_generics as #incur::schemars::JsonSchema>::inline_schema()
                }

                fn schema_name() -> ::std::borrow::Cow<'static, str> {
                    <#helper_ident #type_generics as #incur::schemars::JsonSchema>::schema_name()
                }

                fn schema_id() -> ::std::borrow::Cow<'static, str> {
                    <#helper_ident #type_generics as #incur::schemars::JsonSchema>::schema_id()
                }

                fn json_schema(
                    generator: &mut #incur::schemars::SchemaGenerator,
                ) -> #incur::schemars::Schema {
                    <#helper_ident #type_generics as #incur::schemars::JsonSchema>::json_schema(generator)
                }
            }
        };
    })
}

fn incur_path() -> (TokenStream2, String) {
    match crate_name("incur") {
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{name}");
            (quote!(::#ident), format!("::{name}"))
        }
        Ok(FoundCrate::Itself) | Err(_) => (quote!(::incur), "::incur".to_owned()),
    }
}
