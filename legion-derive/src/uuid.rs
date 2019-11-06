//! PrefabData Implementation

use quote::quote;
use std::{collections::HashMap, fs::OpenOptions, path::Path};
use syn::{parse_macro_input, AttributeArgs, DeriveInput, Ident, ItemStruct};
use uuid::Uuid;

type UuidMap = HashMap<String, u128>;
/*
fn get_uuid(ast: &DeriveInput) -> Uuid {
    let uuid_map_path = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("uuid.ron");
    let uuid_map_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(uuid_map_path)
        .unwrap();

    let item_data = &ast.data;
    let item = parse_macro_input!(item_data as Item);

    let uuid_map: Result<UuidMap, ron::de::Error> = ron::de::from_reader(&uuid_map_file);
    match uuid_map {
        Ok(map) => {
            panic!("GO");
        }
        Err(_) => {
            let mut map = UuidMap::with_capacity(1);

            panic!("tem = {:?}", item);
            // map.insert();
        }
    }

    Uuid::new_v4()
}*/

pub fn impl_uuid(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let parser_copy = input.clone();
    let ast: DeriveInput = parse_macro_input!(input as DeriveInput);

    // Generate a UUID for this type
    let component_name = &ast.ident;

    // Load uuid from file, if none exists generate a new one for this type name
    let uuid = Uuid::new_v4();
    let uuid_bytes: u128 = uuid.as_u128();

    let item = parse_macro_input!(parser_copy as ItemStruct);

    let span = item.ident.span();
    /*
    println!(
        "file {}:{}:{}:{}",
        span.source_file().path().display(),
        span.start().line,
        span.start().column,
        item.ident.to_string()
    );*/

    let res = quote! {
        impl legion::Uuid for #component_name {
            fn uuid() -> uuid::Uuid {
                uuid::Uuid::from_u128(#uuid_bytes)
            }
        }
    };

    res.into()
}
