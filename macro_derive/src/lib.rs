extern crate proc_macro;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Ident;
use quote::quote;
use syn::{Attribute, ItemStruct, LitStr, Path, Result, parse_macro_input};

fn parse_game_event_attr(attrs: &[Attribute], attr_name: &str) -> Result<Option<Path>> {
    for attr in attrs {
        if !attr.path().is_ident(attr_name) {
            continue;
        }

        let mut crate_path: Option<Path> = None;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate_path") {
                let value: Path = meta.value()?.parse()?;
                crate_path = Some(value);
                Ok(())
            } else {
                Err(meta.error("expected `crate_path = <path>`"))
            }
        })?;

        return Ok(crate_path);
    }

    Ok(None)
}

fn type_ident(ty: &syn::Type) -> Option<Ident> {
    match ty {
        syn::Type::Path(type_path) if type_path.qself.is_none() => {
            // Get the last segment of the path (e.g., for `std::string::String`, it's "String")
            type_path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.clone())
        }
        _ => None,
    }
}

#[proc_macro_derive(GameEvent, attributes(game_event))]
pub fn game_event_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as ItemStruct);
    let attrs = &ast.attrs;

    let crate_path = match parse_game_event_attr(attrs, "game_event") {
        Ok(Some(p)) => p,
        _ => {
            match crate_name("demoinfocs2_lite").expect("`demoinfocs2_lite` is not a dependency") {
                FoundCrate::Itself => syn::parse_quote!(crate),
                FoundCrate::Name(name) => {
                    let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
                    syn::parse_quote!(::#ident)
                }
            }
        }
    };

    let ident_msg = &ast.ident;
    let ident_eid = Ident::new(&format!("{ident_msg}SerializerState"), ident_msg.span());

    let fields = ast.fields.iter().map(|f| {
        let field_name = &f.ident;
        quote! {
            #field_name: u32,
        }
    });

    let factory = ast.fields.iter().map(|f| {
        let field_name = &f.ident;
        quote! {
            #field_name: keys.iter()
                .find(|(_, name)| name == stringify!(#field_name))
                .map(|(value, _)| *value)
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Missing field: {}", stringify!(#field_name))
                ))?,
        }
    });

    let serializer = ast.fields.iter().map(|f| {
        let field_name = &f.ident;
        let field_type = &f.ty;
        let ident = type_ident(field_type).expect("Unsupported field type in GameEvent");

        let type_field = if ident == "bool" {
            quote! {val_bool}
        } else if ident == "u8" {
            quote! {val_byte.map(|v| v as u8)}
        } else if ident == "i16" || ident == "u16" {
            quote! {val_short.map(|v| v as #field_type)}
        } else if ident == "i32" || ident == "u32" {
            quote! {val_long.map(|v| v as #field_type)}
        } else if ident == "u64" {
            quote! {val_uint64}
        } else if ident == "f32" {
            quote! {val_float}
        } else if ident == "String" {
            quote! {val_string.map(|v| v.to_string())}
        } else {
            panic!(
                "Unsupported field type: {} for field: {} in GameEvent",
                quote!(#field_type),
                quote!(#field_name)
            );
        };

        quote! {
            if i as u32 == self.#field_name {
                v.#field_name = k.#type_field.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Missing or wrong type for key: {}", stringify!(#field_name)),
                    )
                })?;
            }
        }
    });

    quote! {
        impl #crate_path::event::Event for #ident_msg {}

        struct #ident_eid {
            #( #fields )*
        }

        impl #ident_msg {
            fn factory(keys: &#crate_path::game_event::derive::ListKeysT) -> Result<Box<dyn #crate_path::game_event::derive::GameEventSerializer>, std::io::Error> {
                Ok(Box::new(#ident_eid {
                    #( #factory )*
                }))
            }
        }

        impl #crate_path::game_event::derive::GameEventSerializer for #ident_eid {
            fn parse_and_dispatch_event(&self, keys: Vec<#crate_path::game_event::derive::KeyT>, event_manager: &mut #crate_path::event::EventManager, state: &#crate_path::CsDemoParserState) -> Result<(), std::io::Error> {
                let mut v = #ident_msg::default();

                for (i, k) in keys.into_iter().enumerate() {
                    #( #serializer )*
                }

                event_manager.notify_listeners(v, state)
            }
        }
    }
    .into()
}

#[proc_macro_derive(EntityClass, attributes(entity))]
pub fn entity_class_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as ItemStruct);
    let attrs = &ast.attrs;

    let crate_path = match parse_game_event_attr(attrs, "entity") {
        Ok(Some(p)) => p,
        _ => {
            match crate_name("demoinfocs2_lite").expect("`demoinfocs2_lite` is not a dependency") {
                FoundCrate::Itself => syn::parse_quote!(crate),
                FoundCrate::Name(name) => {
                    let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
                    syn::parse_quote!(::#ident)
                }
            }
        }
    };

    let ident = &ast.ident;

    let fields = ast
        .fields
        .iter()
        .filter_map(|f| {
            let field_ident = f.ident.as_ref()?;
            let getter_ident = Ident::new(&format!("{field_ident}_get"), field_ident.span());
            let (attr_name, on_changed) = get_entity_attr_name(&f.attrs);
            let attr_name = attr_name?;

            Some((field_ident, getter_ident, attr_name, on_changed))
        })
        .collect::<Box<_>>();

    let getters = fields.iter().map(|(field_ident, getter_ident, _, _)| {
        quote! {
            fn #getter_ident(e: &mut #ident) -> &mut dyn std::any::Any {
                &mut e.#field_ident
            }
        }
    });

    let match_branches = fields
        .iter()
        .map(|(_, getter_ident, attr_name, on_changed)| {
            // TODO: perfect hash
            if let Some(on_changed) = on_changed {
                quote! {
                    #attr_name => (Some(#getter_ident as fn(&mut #ident) -> &mut dyn std::any::Any), Some(#on_changed as fn(&mut #ident) -> Result<(), std::io::Error>)),
                }
            } else {
                quote! {
                    #attr_name => (Some(#getter_ident as fn(&mut #ident) -> &mut dyn std::any::Any), None),
                }
            }
        });

    quote! {
        impl #crate_path::entity::serializer::EntityField for #ident {
            fn new() -> Self {
                Self::default()
            }
        }

        impl #ident {
            pub fn new_serializer(
                serializers: Vec<(&str, std::sync::Arc<dyn #crate_path::entity::serializer::EntitySerializer>)>,
            ) -> std::sync::Arc<dyn #crate_path::entity::serializer::EntityClassSerializer> {
                #( #getters )*

                let new_serializers = serializers
                    .into_iter()
                    .map(|(n, s)| {
                        let (getter, callback) = match n {
                            #( #match_branches )*
                            _ => (None, None),
                        };

                        (
                            s,
                            getter,
                            callback,
                        )
                    })
                    .collect();

                std::sync::Arc::new(#crate_path::entity::serializer::CustomEntitySerializer::<Self>::new(new_serializers))
            }
        }
    }
    .into()
}

fn get_entity_attr_name(attrs: &[Attribute]) -> (Option<String>, Option<Path>) {
    for attr in attrs {
        if !attr.path().is_ident("entity") {
            continue;
        }

        let mut name: Option<String> = None;
        let mut on_changed: Option<Path> = None;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let lit: LitStr = meta.value()?.parse()?;
                name = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("on_changed") {
                let value: Path = meta.value()?.parse()?;
                on_changed = Some(value);
                Ok(())
            } else {
                Err(meta.error("unsupported key for #[entity]"))
            }
        })
        .unwrap();

        return (Some(name.unwrap()), on_changed);
    }

    (None, None)
}
