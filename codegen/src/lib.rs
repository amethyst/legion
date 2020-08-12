extern crate proc_macro;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, Attribute, Expr, GenericArgument, Generics, Ident, Index,
    ItemFn, Lit, Meta, PathArguments, Signature, Type, Visibility,
};

/// Wraps a function in a system, and generates a new function which constructs that system.
///
/// There are three types of systems: `simple` (default), `for_each` and `par_for_each`.
/// By default, the system macro will create a new function named `<attributed_fn_name>_system`
/// which can be called to construct the system.
///
/// # Examples
///
/// By default, the wrapped function is called once each time the system runs.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::Schedule;
/// #[system]
/// fn hello_world() {
///    println!("hello world");
/// }
///
/// Schedule::builder()
///     .add_system(hello_world_system())
///     .build();
/// ```
///
/// The function can request resources with reference parameters marked with
/// the `#[resource]` attribute.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::Schedule;
/// # struct Person { name: String }
/// #[system]
/// fn hello_world(#[resource] person: &Person) {
///    println!("hello, {}", person.name);
/// }
/// ```
///
/// Systems can also request a world or command buffer.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::{Schedule, systems::CommandBuffer, world::SubWorld};
/// # struct Person { name: &'static str }
/// #[system]
/// fn create_entity(world: &mut SubWorld, cmd: &mut CommandBuffer) {
///    cmd.push((1usize, false, Person { name: "Jane Doe" }));
/// }
/// ```
///
/// Systems can declare access to component types with the `#[read_component]` and
/// `#[write_component]` attributes.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::{Schedule, world::SubWorld, Read, Write, IntoQuery};
/// # struct Time;
/// #[system]
/// #[read_component(usize)]
/// #[write_component(bool)]
/// fn run_query(world: &mut SubWorld) {
///     let mut query = <(Read<usize>, Write<bool>)>::query();
///     for (a, b) in query.iter_mut(world) {
///         println!("{} {}", a, b);
///     }
/// }
/// ```
///
/// `for_each` and `par_for_each` system types can be used to implement the query for you.
/// References will be interpreted as `Read<T>` and `Write<T>`, while options of references
/// (e.g. `Option<&Position>`) will be interpreted as `TryRead<T>` and `TryWrite<T>`. You can
/// request the entity ID via a `&Entity` parameter.
///
/// ```
/// # use legion_codegen::system;
/// # struct Position { x: f32 }
/// # struct Velocity { x: f32 }
/// # struct Time { seconds: f32 }
/// #[system(for_each)]
/// fn update_positions(pos: &mut Position, vel: &Velocity, #[resource] time: &Time) {
///     pos.x += vel.x * time.seconds;
/// }
/// ```
///
/// `for_each` and `par_for_each` systems can request attitional filters for their query via the
/// `#[filter]` attribute.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::maybe_changed;
/// # struct Position { x: f32 }
/// # struct Velocity { x: f32 }
/// # struct Time { seconds: f32 }
/// #[system(for_each)]
/// #[filter(maybe_changed::<Position>())]
/// fn update_positions(pos: &mut Position, vel: &Velocity, #[resource] time: &Time) {
///     pos.x += vel.x * time.seconds;
/// }
/// ```
///
/// Systems can contain their own state. Add a reference marked with the `#[state]` parameter to
/// your function. This state will be initialized when you construct the system.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::Schedule;
/// #[system]
/// fn stateful(#[state] counter: &mut usize) {
///     *counter += 1;
///     println!("state: {}", counter);
/// }
///
/// Schedule::builder()
///      // initialize state when you construct the system
///     .add_system(stateful_system(5usize))
///     .build();
/// ```
///
/// Systems can contain generic parameters.
///
/// ```
/// # use legion_codegen::system;
/// # use legion::{storage::Component, Schedule};
/// # use std::fmt::Debug;
/// # #[derive(Debug)]
/// # struct Position;
/// #[system(for_each)]
/// fn print_component<T: Component + Debug>(component: &T) {
///     println!("{:?}", component);
/// }
///
/// Schedule::builder()
///      // supply generic parameters when constructing the system
///     .add_system(print_component_system::<Position>())
///     .build();
/// ```
#[proc_macro_attribute]
pub fn system(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = if attr.is_empty() {
        SystemAttr::default()
    } else {
        let meta = parse_macro_input!(attr as Meta);
        match SystemAttr::parse_meta(&meta) {
            Ok(attr) => attr,
            Err(error) => return error.emit(),
        }
    };

    let mut input = parse_macro_input!(item as ItemFn);
    let mut config = match Config::parse(attr, &mut input) {
        Ok(config) => config,
        Err(error) => return error.emit(),
    };
    let system_constructor = config.generate();

    let output = quote! {
        #system_constructor
        #[allow(dead_code)]
        #input
    };

    TokenStream::from(output)
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("system types must be one of `simple`, `for_each` or `par_for_each`")]
    UnexpectedSystemType(Span),
    #[error("duplicate system constructor function name")]
    DuplicateConstructorName,
    #[error("duplicate system type")]
    DuplicateSystemType,
    #[error("invalid key")]
    InvalidKey(Span),
    #[error("system functions must not recieve self")]
    SelfNotAllowed,
    #[error("option arguments must contain a component reference")]
    InvalidOptionArgument(Span),
    #[error(
        "system function parameters must be `CommandBuffer` or `SubWorld` references, \
[optioned] component references, state references, or resource references"
    )]
    InvalidArgument(Span),
    #[error("expected component type")]
    ExpectedComponentType(Span),
    #[error("expected filter expression")]
    ExpectedFilterExpression(Span),
}

impl Error {
    fn span(&self) -> Span {
        match self {
            Error::UnexpectedSystemType(span) => *span,
            Error::InvalidKey(span) => *span,
            Error::InvalidOptionArgument(span) => *span,
            Error::InvalidArgument(span) => *span,
            _ => Span::call_site(),
        }
    }

    fn emit(&self) -> TokenStream {
        let message = format!("{}", self);
        let tokens = quote_spanned!(self.span() => compile_error!(#message));
        TokenStream::from(tokens)
    }
}

#[derive(Default)]
struct SystemAttr {
    constructor_name: Option<Lit>,
    system_type: Option<SystemType>,
}

impl SystemAttr {
    fn new(constructor_name: Option<Lit>, system_type: Option<SystemType>) -> Self {
        Self {
            constructor_name,
            system_type,
        }
    }

    fn parse_meta(meta: &Meta) -> Result<Self, Error> {
        let result = match meta {
            Meta::Path(path) => {
                let ident = path.get_ident().expect("expected system type");
                if ident == "for_each" {
                    Self::new(None, Some(SystemType::ForEach))
                } else if ident == "par_for_each" {
                    Self::new(None, Some(SystemType::ParForEach))
                } else if ident == "simple" {
                    Self::new(None, Some(SystemType::Simple))
                } else {
                    return Err(Error::UnexpectedSystemType(ident.span()));
                }
            }
            Meta::List(items) => {
                let mut n = None;
                let mut s = None;
                for item in &items.nested {
                    let Self {
                        constructor_name,
                        system_type,
                    } = match item {
                        syn::NestedMeta::Meta(meta) => Self::parse_meta(&meta)?,
                        syn::NestedMeta::Lit(_) => panic!("unexpected literal"),
                    };
                    if let Some(constructor_name) = constructor_name {
                        if n.replace(constructor_name).is_some() {
                            return Err(Error::DuplicateConstructorName);
                        }
                    }
                    if let Some(system_type) = system_type {
                        if s.replace(system_type).is_some() {
                            return Err(Error::DuplicateSystemType);
                        }
                    }
                }
                Self::new(n, s)
            }
            Meta::NameValue(name_value) => match name_value.path.get_ident().map(|ident| ident) {
                Some(ident) if ident == "ctor" => Self::new(Some(name_value.lit.clone()), None),
                Some(ident) => return Err(Error::InvalidKey(ident.span())),
                _ => return Err(Error::InvalidKey(Span::call_site())),
            },
        };

        Ok(result)
    }
}

struct Sig {
    ident: Ident,
    parameters: Vec<Parameter>,
    query: Vec<Type>,
    read_resources: Vec<Type>,
    write_resources: Vec<Type>,
    state_args: Vec<Type>,
    generics: Generics,
}

impl Sig {
    fn parse(item: &mut Signature) -> Result<Self, Error> {
        let mut parameters = Vec::new();
        let mut query = Vec::<Type>::new();
        let mut read_resources = Vec::new();
        let mut write_resources = Vec::new();
        let mut state_args = Vec::new();
        for param in &mut item.inputs {
            match param {
                syn::FnArg::Receiver(_) => return Err(Error::SelfNotAllowed),
                syn::FnArg::Typed(arg) => match arg.ty.as_ref() {
                    Type::Path(ty_path) => {
                        let ident = &ty_path.path.segments[0].ident;
                        if ident == "Option" {
                            match &ty_path.path.segments[0].arguments {
                                PathArguments::AngleBracketed(bracketed) => {
                                    let arg = bracketed.args.iter().next().unwrap();
                                    match arg {
                                        GenericArgument::Type(ty) => match ty {
                                            Type::Reference(ty) => {
                                                let mutable = ty.mutability.is_some();
                                                parameters.push(Parameter::Component(query.len()));
                                                let elem = &ty.elem;
                                                if mutable {
                                                    query.push(
                                                        parse_quote!(::legion::TryWrite<#elem>),
                                                    );
                                                } else {
                                                    query.push(
                                                        parse_quote!(::legion::TryRead<#elem>),
                                                    );
                                                }
                                            }
                                            _ => {
                                                return Err(Error::InvalidOptionArgument(
                                                    ident.span(),
                                                ))
                                            }
                                        },
                                        _ => panic!(),
                                    }
                                }
                                _ => panic!(),
                            }
                        } else {
                            return Err(Error::InvalidArgument(ident.span()));
                        }
                    }
                    Type::Reference(ty)
                        if is_type(&ty.elem, &["CommandBuffer"])
                            || is_type(&ty.elem, &["legion", "CommandBuffer"])
                            || is_type(&ty.elem, &["legion", "systems", "CommandBuffer"]) =>
                    {
                        if ty.mutability.is_some() {
                            parameters.push(Parameter::CommandBufferMut);
                        } else {
                            parameters.push(Parameter::CommandBuffer);
                        }
                    }
                    Type::Reference(ty)
                        if is_type(&ty.elem, &["SubWorld"])
                            || is_type(&ty.elem, &["legion", "SubWorld"])
                            || is_type(&ty.elem, &["legion", "world", "SubWorld"]) =>
                    {
                        if ty.mutability.is_some() {
                            parameters.push(Parameter::SubWorldMut);
                        } else {
                            parameters.push(Parameter::SubWorld);
                        }
                    }
                    Type::Reference(ty)
                        if is_type(&ty.elem, &["Entity"])
                            || is_type(&ty.elem, &["legion", "Entity"])
                            || is_type(&ty.elem, &["legion", "world", "Entity"]) =>
                    {
                        parameters.push(Parameter::Component(query.len()));
                        query.push(parse_quote!(::legion::Entity));
                    }
                    Type::Reference(ty) => {
                        let mutable = ty.mutability.is_some();
                        let resource = Self::find_remove_arg_attr(&mut arg.attrs);
                        match resource {
                            Some(ArgAttr::Resource) => {
                                if mutable {
                                    parameters.push(Parameter::ResourceMut(write_resources.len()));
                                    write_resources.push(ty.elem.as_ref().clone());
                                } else {
                                    parameters.push(Parameter::Resource(read_resources.len()));
                                    read_resources.push(ty.elem.as_ref().clone());
                                }
                            }
                            Some(ArgAttr::State) => {
                                if mutable {
                                    parameters.push(Parameter::StateMut(state_args.len()));
                                } else {
                                    parameters.push(Parameter::State(state_args.len()));
                                }
                                state_args.push(ty.elem.as_ref().clone());
                            }
                            None => {
                                parameters.push(Parameter::Component(query.len()));
                                let elem = &ty.elem;
                                if mutable {
                                    query.push(parse_quote!(::legion::Write<#elem>));
                                } else {
                                    query.push(parse_quote!(::legion::Read<#elem>));
                                }
                            }
                        }
                    }
                    _ => return Err(Error::InvalidArgument(Span::call_site())),
                },
            }
        }

        Ok(Self {
            ident: item.ident.clone(),
            generics: item.generics.clone(),
            parameters,
            query,
            read_resources,
            write_resources,
            state_args,
        })
    }

    fn find_remove_arg_attr(attributes: &mut Vec<Attribute>) -> Option<ArgAttr> {
        for i in (0..attributes.len()).rev() {
            match attributes[i].path.get_ident() {
                Some(ident) if ident == "resource" => {
                    attributes.remove(i);
                    return Some(ArgAttr::Resource);
                }
                Some(ident) if ident == "state" => {
                    attributes.remove(i);
                    return Some(ArgAttr::State);
                }
                _ => {}
            }
        }
        None
    }
}

enum ArgAttr {
    Resource,
    State,
}

fn is_type(ty: &Type, segments: &[&str]) -> bool {
    if let Type::Path(path) = ty {
        segments
            .iter()
            .zip(path.path.segments.iter())
            .all(|(a, b)| b.ident == *a)
    } else {
        false
    }
}

#[derive(Copy, Clone, PartialEq)]
enum SystemType {
    Simple,
    ForEach,
    ParForEach,
}

impl SystemType {
    fn requires_query(&self) -> bool {
        match self {
            SystemType::Simple => false,
            SystemType::ForEach => true,
            SystemType::ParForEach => true,
        }
    }
}

enum Parameter {
    CommandBuffer,
    CommandBufferMut,
    SubWorld,
    SubWorldMut,
    Component(usize),
    Resource(usize),
    ResourceMut(usize),
    State(usize),
    StateMut(usize),
}

struct Config {
    attr: SystemAttr,
    visibility: Visibility,
    read_components: Vec<Type>,
    write_components: Vec<Type>,
    filters: Vec<Expr>,
    signature: Sig,
}

impl Config {
    fn parse(attr: SystemAttr, item: &mut ItemFn) -> Result<Self, Error> {
        // parse attributes, extract read/write component/resource and filters
        let mut to_remove = Vec::new();
        let mut read_components = Vec::new();
        let mut write_components = Vec::new();
        let mut filters = Vec::new();
        for (i, attribute) in item.attrs.iter().enumerate() {
            if let Some(ident) = attribute.path.get_ident() {
                if ident == "read_component" {
                    let component = attribute
                        .parse_args()
                        .map_err(|_| Error::ExpectedComponentType(ident.span()))?;
                    read_components.push(component);
                    to_remove.push(i);
                }
                if ident == "write_component" {
                    let component = attribute
                        .parse_args()
                        .map_err(|_| Error::ExpectedComponentType(ident.span()))?;
                    write_components.push(component);
                    to_remove.push(i);
                }
                if ident == "filter" {
                    let filter = attribute
                        .parse_args()
                        .map_err(|_| Error::ExpectedFilterExpression(ident.span()))?;
                    filters.push(filter);
                    to_remove.push(i);
                }
            }
        }

        // remove helper attributes
        for i in to_remove.iter().rev() {
            item.attrs.remove(*i);
        }

        // parse signature, extract cmd, world, components and resources
        let signature = Sig::parse(&mut item.sig)?;

        Ok(Config {
            attr,
            visibility: item.vis.clone(),
            read_components,
            write_components,
            filters,
            signature,
        })
    }

    fn generate(&mut self) -> impl ToTokens {
        let Self {
            attr,
            visibility,
            read_components,
            write_components,
            filters,
            signature,
        } = self;

        let system_type = attr.system_type.unwrap_or(SystemType::Simple);

        // validation
        if !signature.query.is_empty() && system_type == SystemType::Simple {
            panic!("simple systems cannot contain component references, consider using `#[system(for_each)]`");
        }

        if signature.query.is_empty() && system_type != SystemType::Simple {
            panic!("for_each and par_for_each systems require at least one component parameter");
        }

        if signature.generics.lifetimes().next().is_some() {
            panic!("system functions must not contain lifetime generic parameters");
        }

        if system_type == SystemType::ParForEach {
            if signature
                .parameters
                .iter()
                .any(|param| matches!(param, Parameter::SubWorldMut))
            {
                panic!("par_for_each systems cannot accept mutable world references");
            }
            if signature
                .parameters
                .iter()
                .any(|param| matches!(param, Parameter::ResourceMut(_)))
            {
                panic!("par_for_each systems cannot accept mutable resource references");
            }
            if signature
                .parameters
                .iter()
                .any(|param| matches!(param, Parameter::CommandBufferMut))
            {
                panic!("par_for_each systems cannot accept mutable command buffer references");
            }
        }

        // declare query
        let query = if system_type.requires_query() {
            let views = &signature.query;
            quote! {
                .with_query(
                    <(#(#views),*)>::query()
                    #(.filter(#filters))*
                )
            }
        } else {
            quote!()
        };

        // construct function arguments
        let has_query = !signature.query.is_empty();
        let single_resource =
            (signature.read_resources.len() + signature.write_resources.len()) == 1;
        let mut call_params = Vec::new();
        let mut fn_params = Vec::new();
        let mut world = None;
        for param in &signature.parameters {
            match param {
                Parameter::CommandBuffer => call_params.push(quote!(cmd)),
                Parameter::CommandBufferMut => call_params.push(quote!(cmd)),
                Parameter::SubWorld => {
                    if has_query {
                        call_params.push(quote!(&world));
                    } else {
                        call_params.push(quote!(world));
                    }
                    world = Some(quote! {
                        let (mut for_query, world) = world.split_for_query(query);
                        let for_query = &mut for_query;
                    });
                }
                Parameter::SubWorldMut => {
                    if has_query {
                        call_params.push(quote!(&mut world));
                    } else {
                        call_params.push(quote!(world));
                    }
                    world = Some(quote! {
                        let (mut for_query, mut world) = world.split_for_query(query);
                        let for_query = &mut for_query;
                    });
                }
                Parameter::Component(_) if signature.query.len() == 1 => {
                    call_params.push(quote!(components))
                }
                Parameter::Component(idx) => {
                    let idx = Index::from(*idx);
                    call_params.push(quote!(components.#idx));
                }
                Parameter::Resource(_) if single_resource => call_params.push(quote!(&*resources)),
                Parameter::ResourceMut(_) if single_resource => {
                    call_params.push(quote!(&mut *resources))
                }
                Parameter::Resource(idx) => {
                    let idx = Index::from(*idx);
                    call_params.push(quote!(&*resources.#idx));
                }
                Parameter::ResourceMut(idx) => {
                    let idx = Index::from(*idx + signature.read_resources.len());
                    call_params.push(quote!(&mut *resources.#idx));
                }
                Parameter::State(idx) => {
                    let arg_name = format_ident!("state_{}", idx);
                    let arg_type = &signature.state_args[*idx];
                    call_params.push(quote!(&#arg_name));
                    fn_params.push(quote!(#arg_name: #arg_type));
                }
                Parameter::StateMut(idx) => {
                    let arg_name = format_ident!("state_{}", idx);
                    let arg_type = &signature.state_args[*idx];
                    call_params.push(quote!(&mut #arg_name));
                    fn_params.push(quote!(mut #arg_name: #arg_type));
                }
            }
        }

        // construct function body
        let fn_id = &signature.ident;
        let type_params = signature
            .generics
            .type_params()
            .map(|param| param.ident.clone());
        let fn_call = quote!(#fn_id::<#(#type_params),*>(#(#call_params),*););
        let world = world.unwrap_or_else(|| quote!(let for_query = world;));
        let body = match system_type {
            SystemType::Simple => fn_call,
            SystemType::ForEach => quote! {
                #world
                query.for_each_mut(for_query, |components| {
                    #fn_call
                });
            },
            SystemType::ParForEach => quote! {
                #world
                query.par_for_each_mut(for_query, |components| {
                    #fn_call
                });
            },
        };

        // construct our system
        let system_name = fn_id.to_string();
        let generic_parameter_names = if signature.generics.type_params().next().is_some() {
            {
                let param_names = signature
                    .generics
                    .type_params()
                    .map(|param| param.ident.clone())
                    .collect::<Vec<_>>();
                quote! {
                    let generic_names = "<".to_owned() + &[#(std::any::type_name::<#param_names>()),*].join(", ") + ">";
                }
            }
        } else {
            quote!(let generic_names = "";)
        };
        let read_resources = &signature.read_resources;
        let write_resources = &signature.write_resources;
        let builder = quote! {
            use legion::IntoQuery;
            #generic_parameter_names
            ::legion::systems::SystemBuilder::new(format!("{}{}", #system_name, generic_names))
                #(.read_component::<#read_components>())*
                #(.write_component::<#write_components>())*
                #(.read_resource::<#read_resources>())*
                #(.write_resource::<#write_resources>())*
                #query
                .build(move |cmd, world, resources, query| {
                    #body
                })
        };

        // construct our system constructor function
        let constructor_name = if let Some(name) = &attr.constructor_name {
            let (name, span) = match name {
                Lit::Str(name) => (name.value(), name.span()),
                Lit::Char(name) => (name.value().to_string(), name.span()),
                Lit::Verbatim(name) => (name.to_string(), name.span()),
                _ => panic!("invalid system constructor name"),
            };
            Ident::new(&name, span)
        } else {
            format_ident!("{}_system", fn_id)
        };

        let generic_params = signature.generics.params.clone();
        let where_clause = signature.generics.make_where_clause();

        quote! {
            #visibility fn #constructor_name<#generic_params>(#(#fn_params),*) -> impl ::legion::systems::Runnable
            #where_clause
            {
                #builder
            }
        }
    }
}
