use syn::{
    punctuated::Punctuated, token::Comma, Data, DataStruct, DeriveInput, Field, Fields,
    FieldsNamed, FieldsUnnamed, GenericArgument, Ident, Lifetime, PathArguments, Type, TypePath,
    WhereClause, WherePredicate,
};

/// Used to `#[derive]` the trait `IntoQuery`.
///
/// You need to have the following items included in the current scope:
///
/// * `IntoQuery`
/// * `Query`

pub fn impl_system_resources(ast: &DeriveInput) -> proc_macro2::TokenStream {
    let name = &ast.ident;
    let mut generics = ast.generics.clone();

    let (_fetch_return, tys) = gen_from_body(&ast.data, name);
    let tys = &tys;
    // Assumes that the first lifetime is the fetch lt
    let def_fetch_lt = ast
        .generics
        .lifetimes()
        .next()
        .expect("There has to be at least one lifetime");
    let impl_fetch_lt = &def_fetch_lt.lifetime;

    {
        let where_clause = generics.make_where_clause();
        constrain_system_data_types(where_clause, impl_fetch_lt, tys);
    }
    // Reads and writes are taken from the same types,
    // but need to be cloned before.

    let (_impl_generics, ty_generics, _where_clause) = generics.split_for_impl();

    let system_resources_info = system_resources_info_from_ast(&ast.data);
    let (write_resources, read_resources) =
        partition_resources(&system_resources_info.resource_types);

    let mut resource_types_iter = system_resources_info
        .resource_types
        .iter()
        .cloned()
        .rev()
        .map(|resource_info| (resource_info.resource_type, resource_info.mutable));

    let (mut cons_concat_arg_type, mut cons_concat_expr) = resource_types_iter.next().map_or(
        (quote! { () }, quote! { () }),
        |(resource_type, mutable)| {
            if mutable {
                (
                    quote! { (Write< #resource_type >, ()) },
                    quote! { (Write< #resource_type >::default(), ()) },
                )
            } else {
                (
                    quote! { (Read< #resource_type >, ()) },
                    quote! { (Read::< #resource_type >::default(), ()) },
                )
            }
        },
    );

    for (resource_type, mutable) in resource_types_iter {
        cons_concat_arg_type = if mutable {
            quote! { (Write< #resource_type >, #cons_concat_arg_type) }
        } else {
            quote! { (Read< #resource_type >, #cons_concat_arg_type) }
        };

        cons_concat_expr = if mutable {
            quote! { (Write::< #resource_type >::default(), #cons_concat_expr) }
        } else {
            quote! { (Read::< #resource_type >::default(), #cons_concat_expr) }
        };
    }

    let field_names = system_resources_info.field_names.clone();
    let resource_fetches = system_resources_info
        .resource_types
        .iter()
        .cloned()
        .map(|resource_info| (resource_info.resource_type, resource_info.mutable))
        .zip(field_names.iter())
        .map(|((resource_type, mutable), field_name)| {
            if mutable {
                quote! {
                    #field_name: {
                        let type_id = &::legion::systems::ResourceTypeId::of::< #resource_type >();
                        resources.get(&type_id).unwrap().get_mut::< #resource_type >().unwrap()
                    },
                }
            } else {
                quote! {
                    #field_name: {
                        let type_id = &::legion::systems::ResourceTypeId::of::< #resource_type >();
                        resources.get(&type_id).unwrap().get::< #resource_type >().unwrap()
                    },
                }
            }
        })
        .collect::<Vec<_>>();

    quote! {
        impl #ty_generics SystemResources<#impl_fetch_lt> for #name #ty_generics {
            type ConsConcatArg = #cons_concat_arg_type;

            fn resources() -> Self::ConsConcatArg {
                #cons_concat_expr
            }

            fn read_resource_type_ids() -> Vec<::legion::systems::ResourceTypeId> {
                let mut resource_type_ids = Vec::new();
                #(
                    resource_type_ids.push(::legion::systems::ResourceTypeId::of::< #read_resources >());
                )*
                resource_type_ids
            }

            fn write_resource_type_ids() -> Vec<::legion::systems::ResourceTypeId> {
                let mut resource_type_ids = Vec::new();
                #(
                    resource_type_ids.push(::legion::systems::ResourceTypeId::of::< #write_resources >());
                )*
                resource_type_ids
            }

            unsafe fn fetch_unchecked(resources: & #impl_fetch_lt ::legion::systems::UnsafeResources) -> Self {
                // TODO: support tuple structs
                Self { #(
                    #resource_fetches
                )* }
            }
        }
    }
}

fn collect_field_types(fields: &Punctuated<Field, Comma>) -> Vec<Type> {
    fields.iter().map(|x| x.ty.clone()).collect()
}

fn gen_identifiers(fields: &Punctuated<Field, Comma>) -> Vec<Ident> {
    fields.iter().map(|x| x.ident.clone().unwrap()).collect()
}

/// Adds a `IntoQuery<'lt>` bound on each of the system data types.
fn constrain_system_data_types(clause: &mut WhereClause, fetch_lt: &Lifetime, tys: &[Type]) {
    for ty in tys.iter() {
        let where_predicate: WherePredicate = parse_quote!(#ty : IntoQuery< #fetch_lt >);
        clause.predicates.push(where_predicate);
    }
}

#[derive(Clone)]
enum DataType {
    Struct,
    Tuple,
}

#[derive(Clone)]
struct ResourceInfo {
    mutable: bool,
    resource_type: Type,
}

#[derive(Clone)]
struct SystemResourcesInfo {
    data_type: DataType,
    field_names: Vec<Ident>,
    struct_types: Vec<Type>,
    // Component names (stripped out references from struct_fields).
    resource_types: Vec<ResourceInfo>,
}

fn partition_resources(resource_types: &[ResourceInfo]) -> (Vec<Type>, Vec<Type>) {
    let (write_resources, read_resources) = resource_types
        .iter()
        .cloned()
        .partition::<Vec<_>, _>(|resource_type| resource_type.mutable);

    fn map_type(resource_info: ResourceInfo) -> Type {
        resource_info.resource_type
    }

    let write_resources = write_resources.into_iter().map(map_type).collect();
    let read_resources = read_resources.into_iter().map(map_type).collect();

    (write_resources, read_resources)
}

fn system_resources_info_from_ast(ast: &Data) -> SystemResourcesInfo {
    let (data_type, fields) = match *ast {
        Data::Struct(DataStruct {
            fields: Fields::Named(FieldsNamed { named: ref x, .. }),
            ..
        }) => (DataType::Struct, x),
        Data::Struct(DataStruct {
            fields: Fields::Unnamed(FieldsUnnamed { unnamed: ref x, .. }),
            ..
        }) => (DataType::Tuple, x),
        _ => panic!("Enums are not supported"),
    };

    let field_names = if let DataType::Struct = data_type {
        fields
            .iter()
            .map(|field| {
                field
                    .ident
                    .clone()
                    .expect("Expected a name for a named struct field")
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut struct_types = Vec::with_capacity(fields.len());
    let mut resource_types = Vec::with_capacity(fields.len());

    for field in fields {
        match &field.ty {
            Type::Reference(_) => {
                panic!("Only Fetch or FetchMut are supported");
            }
            Type::Path(type_path) => {
                let resources_type = try_fetch_type(type_path.clone());
                struct_types.push(Type::Path(type_path.clone()));
                resource_types.push(resources_type);
            }
            _ => panic!("Only reference or Option types are supported"),
        }
    }

    SystemResourcesInfo {
        data_type,
        field_names,
        struct_types,
        resource_types,
    }
}

fn try_fetch_type(type_path: TypePath) -> ResourceInfo {
    let last_path_segment = type_path.path.segments.last().unwrap();
    // TODO: support for custom paths, if it's possible to pass with macro attributes
    let mutable = match last_path_segment.ident.to_string().as_ref() {
        "Fetch" => false,
        "FetchMut" => true,
        _ => panic!("meh"),
    };

    let resource_type =
        if let PathArguments::AngleBracketed(generic_arguments) = &last_path_segment.arguments {
            if let GenericArgument::Type(type_arg) = generic_arguments.args.last().unwrap() {
                type_arg.clone()
            } else {
                panic!("Expected a type as the last generic argument for Fetch or FetchMut");
            }
        } else {
            panic!("Expected generic arguments for Fetch or FetchMut");
        };

    ResourceInfo {
        mutable,
        resource_type,
    }
}

fn gen_from_body(ast: &Data, name: &Ident) -> (proc_macro2::TokenStream, Vec<Type>) {
    let (body, fields) = match *ast {
        Data::Struct(DataStruct {
            fields: Fields::Named(FieldsNamed { named: ref x, .. }),
            ..
        }) => (DataType::Struct, x),
        Data::Struct(DataStruct {
            fields: Fields::Unnamed(FieldsUnnamed { unnamed: ref x, .. }),
            ..
        }) => (DataType::Tuple, x),
        _ => panic!("Enums are not supported"),
    };

    let tys = collect_field_types(fields);

    let fetch_return = match body {
        DataType::Struct => {
            let identifiers = gen_identifiers(fields);

            quote! {
                #name {
                    #( #identifiers: IntoQuery::fetch(world) ),*
                }
            }
        }
        DataType::Tuple => {
            let count = tys.len();
            let fetch = vec![quote! { IntoQuery::fetch(world) }; count];

            quote! {
                #name ( #( #fetch ),* )
            }
        }
    };

    (fetch_return, tys)
}
