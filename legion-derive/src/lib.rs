mod uuid;

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

/// EventReader
#[proc_macro_derive(Uuid, attributes(component))]
pub fn derive_uuid(input: TokenStream) -> TokenStream { uuid::impl_uuid(input).into() }
