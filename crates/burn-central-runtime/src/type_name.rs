use syn::{ExprPath, parse_str};
use tynm::TypeParamsFmtOpts;

/// This is not a very robust way to get the function name from a type, and may fail in various ways
/// (i.e. closures), but this is mostly for diagnostic purposes.
pub fn fn_type_name<T>() -> String {
    // Get the type name as a string
    let type_name = &tynm::type_namen_opts::<T>(99, TypeParamsFmtOpts::Std);

    // if there is a trailing :: remove it
    let type_name = if type_name.ends_with("::") {
        &type_name[..type_name.len() - 2]
    } else {
        type_name
    };

    let path: Result<ExprPath, _> = parse_str(type_name);
    if path.is_err() {
        return type_name.to_string();
    }
    let path = path.unwrap();

    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_else(|| panic!("Failed to extract function name from type: {type_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fn_type_name_by_val<T>(_: T) -> String {
        fn_type_name::<T>()
    }

    fn plain_func(_t: i32) {}
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
