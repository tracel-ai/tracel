use syn::{punctuated::Punctuated, Expr, Meta};

pub trait LitMatcher<T> {
    fn match_type(&self) -> T;
}

impl LitMatcher<String> for syn::Lit {
    fn match_type(&self) -> String {
        match self {
            syn::Lit::Str(lit) => lit.value(),
            _ => panic!("Expected a string literal"),
        }
    }
}

impl LitMatcher<bool> for syn::Lit {
    fn match_type(&self) -> bool {
        match self {
            syn::Lit::Bool(lit) => lit.value,
            _ => panic!("Expected a boolean literal"),
        }
    }
}

pub fn get_name_value<T>(args: &Punctuated<Meta, syn::token::Comma>, name: &str) -> Option<T>
where
    syn::Lit: LitMatcher<T>,
{
    args.iter()
        .find(|meta| meta.path().is_ident(name))
        .and_then(|meta| {
            if let Meta::NameValue(meta) = meta {
                if let Expr::Lit(lit) = &meta.value {
                    Some(lit.lit.match_type())
                } else {
                    None
                }
            } else {
                None
            }
        })
}
