use syn::GenericArgument::Type;
use syn::{Expr, ExprPath, parse_str};
use tynm::TypeParamsFmtOpts;

/// Extracts the base name of a function's type (as seen in `std::any::type_name`)
/// using robust parsing via `syn`.
///
/// # Panics
/// Panics if the type name is not a valid Rust path (e.g. closures or anonymous types).
pub fn fn_type_name<T>() -> String {
    // Get the type name as a string
    let type_name = &tynm::type_namen_opts::<T>(99, TypeParamsFmtOpts::Std);

    // if there is a trailing :: remove it
    let type_name = if type_name.ends_with("::") {
        &type_name[..type_name.len() - 2]
    } else {
        type_name
    };

    println!("type_name: {}", type_name);

    // Parse the type name into a syn ItemFn
    let path: Result<ExprPath, _> = parse_str(type_name);
    if let Err(_) = path {
        return type_name.to_string();
    }
    let path = path.unwrap();

    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_else(|| panic!("Failed to extract function name from type: {}", type_name))
}

pub fn fn_type_name_by_val<T>(_: T) -> String {
    fn_type_name::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_func(t: i32) {}
    fn generic_func<T>(_x: T) {}
    mod nested {
        pub fn inner_func() {}
    }

    #[test]
    fn extracts_plain_function_name() {
        let name = fn_type_name_by_val(plain_func);
        assert_eq!(name, "plain_func");
    }

    #[test]
    fn extracts_generic_function_name() {
        let name = fn_type_name_by_val(generic_func::<i32>);
        assert_eq!(name, "generic_func");
    }

    #[test]
    fn extracts_nested_function_name() {
        let name = fn_type_name_by_val(nested::inner_func);
        assert_eq!(name, "inner_func");
    }

    #[test]
    fn extracts_function_pointer_name() {
        let f: fn(i32) = plain_func;
        let name = fn_type_name_by_val(f);
        assert_eq!(name, "fn");
    }
}
