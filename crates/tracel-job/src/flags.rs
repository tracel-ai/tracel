//! The flags of a job's command line, read from its definition: one per field of its input.

use serde_json::{Map, Number, Value};

use crate::JobDefinition;

/// Flag names the runner keeps: a field with one of these names has no flag.
pub const RESERVED: [&str; 3] = ["help", "version", "config"];

/// A flag that sets one field of a job's input.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// The keys of the field, from the top of the input down.
    pub path: Vec<String>,
    /// The values the flag takes.
    pub kind: Kind,
    /// What the field is, from the schema.
    pub help: Option<String>,
    /// The field's value in the example input, or else its schema default.
    pub default: Option<Value>,
    /// Whether the schema requires the field and nothing gives it a default.
    pub required: bool,
}

impl Flag {
    /// The flag's name: the keys of the field joined by dots, each with `_` written `-`.
    pub fn name(&self) -> String {
        let keys: Vec<String> = self.path.iter().map(|key| key.replace('_', "-")).collect();
        keys.join(".")
    }

    /// The flag's argument id: the keys of the field joined by dots, which no key contains.
    pub fn id(&self) -> String {
        self.path.join(".")
    }
}

/// Values a flag can take, each with its description.
pub type Choices = Vec<(Value, Option<String>)>;

/// The values a flag takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// An integer.
    Integer,
    /// A number.
    Float,
    /// A string, taken as given.
    String,
    /// One of these values.
    Choice(Choices),
    /// A JSON literal.
    Json,
}

impl Kind {
    /// The kind of a field whose example value is `value`.
    fn of(value: &Value) -> Self {
        match value {
            Value::Bool(_) => Self::Bool,
            Value::Number(number) if number.is_f64() => Self::Float,
            Value::Number(_) => Self::Integer,
            Value::String(_) => Self::String,
            Value::Null | Value::Array(_) | Value::Object(_) => Self::Json,
        }
    }

    /// Parses a flag's argument as the field's value.
    pub fn parse(&self, argument: &str) -> Result<Value, String> {
        match self {
            Self::Bool => match argument {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err("expected true or false".to_string()),
            },
            Self::Integer => match number(argument) {
                Some(number) if !number.is_f64() => Ok(Value::Number(number)),
                _ => Err("expected an integer".to_string()),
            },
            Self::Float => number(argument)
                .map(Value::Number)
                .ok_or_else(|| "expected a number".to_string()),
            Self::String => Ok(Value::String(argument.to_string())),
            Self::Choice(choices) => choices
                .iter()
                .map(|(value, _)| value)
                .find(|value| scalar_text(value).as_deref() == Some(argument))
                .cloned()
                .ok_or_else(|| "not one of the possible values".to_string()),
            Self::Json => {
                serde_json::from_str(argument).map_err(|error| format!("not JSON: {error}"))
            }
        }
    }

    /// `value` written as the flag's argument, when the flag takes it.
    pub fn render(&self, value: &Value) -> Option<String> {
        let text = match (self, value) {
            (Self::Bool, Value::Bool(value)) => value.to_string(),
            (Self::Integer, Value::Number(number)) => number.to_string(),
            (Self::Float, Value::Number(number)) => float_text(number.as_f64()?),
            (Self::String, Value::String(text)) => text.clone(),
            (Self::Choice(_), value) => scalar_text(value)?,
            (Self::Json, value) => value.to_string(),
            _ => return None,
        };
        self.parse(&text).is_ok().then_some(text)
    }
}

/// `argument` as a JSON number, also read the way Rust reads numbers, such as `.5` or `+3`.
fn number(argument: &str) -> Option<Number> {
    match serde_json::from_str(argument) {
        Ok(Value::Number(number)) => Some(number),
        _ => argument
            .parse::<i64>()
            .map(Number::from)
            .ok()
            .or_else(|| Number::from_f64(argument.parse().ok()?)),
    }
}

/// The text of a string, number or boolean.
pub fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// The shortest decimal text of `value`, or of the `f32` it was widened from: `0.00005` rather
/// than `0.00004999999873689376`.
fn float_text(value: f64) -> String {
    let single = value as f32;
    if f64::from(single) == value {
        single.to_string()
    } else {
        value.to_string()
    }
}

/// The flags of the job `definition` describes, one per field of its input.
///
/// With a schema that describes an object, each field the schema gives a type (a boolean, an
/// integer, a number, a string, or one of the values it lists) is a flag of that type, nested
/// objects are walked into, and the example input gives defaults. Otherwise each field of the
/// example input is a flag typed by its value, which is its default. Either way, a field that is
/// an array, a map, a nullable object or of no single type, or whose example value is `null` or
/// an empty object, takes a JSON literal.
///
/// A field whose key is not made of letters, digits, `_` and `-`, whose flag name is reserved, or
/// whose flag name an earlier field took, has no flag.
pub fn flags(definition: &JobDefinition) -> Vec<Flag> {
    let example = definition.input_example.as_ref();
    let mut flags = Vec::new();
    let schema = definition.input_schema.as_ref().and_then(|root| {
        match shape(root, root, &mut Vec::new()) {
            Shape::Object(object) => Some((root, object)),
            Shape::Leaf(_) => None,
        }
    });
    match (schema, example) {
        (Some((root, object)), _) => SchemaWalk {
            root,
            example,
            flags: &mut flags,
        }
        .object(object, &mut Vec::new(), true, &mut Vec::new()),
        (None, Some(Value::Object(fields))) => example_flags(fields, &mut Vec::new(), &mut flags),
        (None, _) => {}
    }

    let mut names = Vec::new();
    flags.retain(|flag| {
        let name = flag.name();
        let keep = !RESERVED.contains(&name.as_str()) && !names.contains(&name);
        names.push(name);
        keep
    });
    flags
}

/// Whether `key` can be part of a flag name.
fn is_flag_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('-')
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// Adds a flag for each field of the example object `fields` at `path`.
fn example_flags(fields: &Map<String, Value>, path: &mut Vec<String>, flags: &mut Vec<Flag>) {
    for (key, value) in fields {
        if !is_flag_key(key) {
            continue;
        }
        path.push(key.clone());
        match value {
            Value::Object(fields) if !fields.is_empty() => example_flags(fields, path, flags),
            value => flags.push(Flag {
                path: path.clone(),
                kind: Kind::of(value),
                help: None,
                default: Some(value.clone()),
                required: false,
            }),
        }
        path.pop();
    }
}

/// What a schema describes: an object whose fields are walked into, or the values of one flag.
enum Shape<'a> {
    Object(&'a Map<String, Value>),
    Leaf(Kind),
}

/// The schema `schema`'s `$ref` points to within `root`, or `schema` itself when it has none.
/// `None` for a reference outside `root`.
fn follow<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    match reference(schema) {
        Some(reference) => root.pointer(reference.strip_prefix('#')?),
        None => Some(schema),
    }
}

/// The `$ref` of `schema`.
fn reference(schema: &Value) -> Option<&str> {
    schema.get("$ref").and_then(Value::as_str)
}

/// Whether `schema` describes only `null`.
fn is_null(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
        || schema.get("const").is_some_and(Value::is_null)
}

/// The values `schema` lists with `const` or `enum`, with their descriptions, leaving out `null`.
fn listed(schema: &Map<String, Value>) -> Option<Choices> {
    let description = description(schema);
    if let Some(value) = schema.get("const") {
        return Some(vec![(value.clone(), description)]);
    }
    let values = schema.get("enum")?.as_array()?;
    let values: Vec<&Value> = values.iter().filter(|value| !value.is_null()).collect();
    let description = if values.len() == 1 { description } else { None };
    Some(
        values
            .into_iter()
            .map(|value| (value.clone(), description.clone()))
            .collect(),
    )
}

/// The shape of what `schema` describes, following references within `root`.
///
/// `following` holds the references followed to reach `schema`, and gets those followed to
/// reach its shape. A reference already followed, which a recursive type makes, takes a JSON
/// literal.
fn shape<'a>(root: &'a Value, schema: &'a Value, following: &mut Vec<&'a str>) -> Shape<'a> {
    if let Some(reference) = reference(schema) {
        if following.contains(&reference) {
            return Shape::Leaf(Kind::Json);
        }
        following.push(reference);
        return match follow(root, schema) {
            Some(target) => shape(root, target, following),
            None => Shape::Leaf(Kind::Json),
        };
    }
    let Some(schema) = schema.as_object() else {
        return Shape::Leaf(Kind::Json);
    };

    if let Some(values) = listed(schema) {
        return choice(values);
    }
    if let Some(variants) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let values: Vec<&Value> = variants
            .iter()
            .filter(|variant| !is_null(variant))
            .collect();
        // One of a set of constants, such as the unit variants of an enum.
        let constants: Option<Vec<Choices>> = values
            .iter()
            .map(|variant| {
                follow(root, variant)
                    .and_then(Value::as_object)
                    .and_then(listed)
            })
            .collect();
        if let Some(constants) = constants.filter(|constants| !constants.is_empty()) {
            return choice(constants.into_iter().flatten().collect());
        }
        let nullable = values.len() < variants.len();
        return match values[..] {
            [value] => match shape(root, value, following) {
                Shape::Object(_) if nullable => Shape::Leaf(Kind::Json),
                shape => shape,
            },
            _ => Shape::Leaf(Kind::Json),
        };
    }
    if let Some([only]) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        return shape(root, only, following);
    }

    let (types, nullable) = match schema.get("type") {
        Some(Value::String(name)) => (vec![name.as_str()], false),
        Some(Value::Array(names)) => {
            let names: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
            let nullable = names.contains(&"null");
            let types = names.into_iter().filter(|name| *name != "null").collect();
            (types, nullable)
        }
        _ => (Vec::new(), false),
    };
    let has_properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty());
    match types[..] {
        ["boolean"] => Shape::Leaf(Kind::Bool),
        ["integer"] => Shape::Leaf(Kind::Integer),
        ["number"] => Shape::Leaf(Kind::Float),
        ["string"] => Shape::Leaf(Kind::String),
        ["object"] | [] if has_properties && !nullable => Shape::Object(schema),
        _ => Shape::Leaf(Kind::Json),
    }
}

/// A flag that takes one of `values`, or a JSON literal when there are none or one is not a
/// string, number or boolean.
fn choice<'a>(values: Choices) -> Shape<'a> {
    if values.is_empty() || values.iter().any(|(value, _)| scalar_text(value).is_none()) {
        Shape::Leaf(Kind::Json)
    } else {
        Shape::Leaf(Kind::Choice(values))
    }
}

/// The description of what `schema` describes.
fn description(schema: &Map<String, Value>) -> Option<String> {
    schema
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Walks a schema that describes an object, adding a flag for each field.
struct SchemaWalk<'a, 'f> {
    root: &'a Value,
    example: Option<&'a Value>,
    flags: &'f mut Vec<Flag>,
}

impl<'a> SchemaWalk<'a, '_> {
    /// Adds the flags of the fields of `object`, the schema of the object at `path`, reached
    /// through the references `following`. The object is required when `required` is.
    fn object(
        &mut self,
        object: &'a Map<String, Value>,
        path: &mut Vec<String>,
        required: bool,
        following: &mut Vec<&'a str>,
    ) {
        let Some(properties) = object.get("properties").and_then(Value::as_object) else {
            return;
        };
        let required_keys: Vec<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for (key, property) in properties {
            if !is_flag_key(key) {
                continue;
            }
            path.push(key.clone());
            let required = required && required_keys.contains(&key.as_str());
            let followed = following.len();
            match shape(self.root, property, following) {
                Shape::Object(object) => self.object(object, path, required, following),
                Shape::Leaf(kind) => self.leaf(property, path, kind, required),
            }
            following.truncate(followed);
            path.pop();
        }
    }

    /// Adds the flag of the field at `path`, whose schema is `property`.
    fn leaf(&mut self, property: &Value, path: &[String], kind: Kind, required: bool) {
        let default = self
            .example
            .and_then(|example| lookup(example, path))
            .or_else(|| property.get("default"))
            .cloned();
        let help = [Some(property), follow(self.root, property)]
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .find_map(description);
        self.flags.push(Flag {
            path: path.to_vec(),
            kind,
            help,
            required: required && default.is_none(),
            default,
        });
    }
}

/// The value at `path` in `value`.
fn lookup<'v>(value: &'v Value, path: &[String]) -> Option<&'v Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn definition(schema: Option<Value>, example: Option<Value>) -> JobDefinition {
        JobDefinition {
            name: "train".to_string(),
            description: None,
            input_schema: schema,
            input_example: example,
        }
    }

    /// The name, kind and default of each flag.
    fn summary(flags: &[Flag]) -> Vec<(String, Kind, Option<Value>)> {
        flags
            .iter()
            .map(|flag| (flag.name(), flag.kind.clone(), flag.default.clone()))
            .collect()
    }

    fn flag<'f>(flags: &'f [Flag], name: &str) -> &'f Flag {
        flags
            .iter()
            .find(|flag| flag.name() == name)
            .unwrap_or_else(|| panic!("no flag --{name} in {:?}", summary(flags)))
    }

    #[test]
    fn each_leaf_of_the_example_is_a_flag_typed_by_its_value() {
        let flags = flags(&definition(
            None,
            Some(json!({
                "num_epochs": 10,
                "shuffle": true,
                "tag": "baseline",
                "layers": [64, 32],
                "resume_from": null,
                "extra": {},
                "optimizer": {"lr": 0.001, "momentum": 0.0, "betas": {"beta_1": 0.9}}
            })),
        ));

        assert_eq!(
            summary(&flags),
            [
                ("extra".to_string(), Kind::Json, Some(json!({}))),
                ("layers".to_string(), Kind::Json, Some(json!([64, 32]))),
                ("num-epochs".to_string(), Kind::Integer, Some(json!(10))),
                (
                    "optimizer.betas.beta-1".to_string(),
                    Kind::Float,
                    Some(json!(0.9))
                ),
                ("optimizer.lr".to_string(), Kind::Float, Some(json!(0.001))),
                (
                    "optimizer.momentum".to_string(),
                    Kind::Float,
                    Some(json!(0.0))
                ),
                ("resume-from".to_string(), Kind::Json, Some(Value::Null)),
                ("shuffle".to_string(), Kind::Bool, Some(json!(true))),
                ("tag".to_string(), Kind::String, Some(json!("baseline"))),
            ]
        );
        assert_eq!(
            flag(&flags, "optimizer.betas.beta-1").path,
            ["optimizer", "betas", "beta_1"]
        );
        assert_eq!(
            flag(&flags, "optimizer.betas.beta-1").id(),
            "optimizer.betas.beta_1"
        );
        assert!(
            flags
                .iter()
                .all(|flag| !flag.required && flag.help.is_none())
        );
    }

    #[test]
    fn an_input_that_is_not_an_object_has_no_flags() {
        assert!(flags(&definition(None, Some(json!(10)))).is_empty());
        assert!(flags(&definition(None, Some(json!({})))).is_empty());
        assert!(flags(&definition(None, None)).is_empty());
        assert!(flags(&definition(Some(json!({"type": "string"})), None)).is_empty());
    }

    /// A schema in the form schemars writes for a documented configuration.
    fn config_schema() -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Config",
            "type": "object",
            "properties": {
                "epochs": {
                    "description": "Passes over the data.",
                    "type": "integer",
                    "format": "uint32",
                    "minimum": 0
                },
                "seed": {"type": ["integer", "null"], "format": "uint64"},
                "optimizer": {"description": "The optimizer.", "$ref": "#/$defs/Optimizer"},
                "device": {"$ref": "#/$defs/Device"},
                "resume_from": {"anyOf": [{"$ref": "#/$defs/Resume"}, {"type": "null"}]},
                "layers": {"type": "array", "items": {"type": "integer"}},
                "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                "anything": true,
                "verbose": {"type": "boolean", "default": false}
            },
            "required": ["epochs", "optimizer", "device", "layers", "labels"],
            "$defs": {
                "Optimizer": {
                    "type": "object",
                    "properties": {
                        "lr": {"description": "The learning rate.", "type": "number"},
                        "kind": {"type": "string", "enum": ["adam", "sgd"]}
                    },
                    "required": ["lr", "kind"]
                },
                "Device": {
                    "description": "Where to train.",
                    "oneOf": [
                        {"description": "The CPU.", "type": "string", "const": "cpu"},
                        {"description": "The first GPU.", "type": "string", "const": "gpu"}
                    ]
                },
                "Resume": {
                    "type": "object",
                    "properties": {"experiment": {"type": "integer"}},
                    "required": ["experiment"]
                }
            }
        })
    }

    #[test]
    fn a_schema_types_each_field_and_gives_its_help() {
        let flags = flags(&definition(Some(config_schema()), None));

        let kinds: Vec<(String, Kind)> = flags
            .iter()
            .map(|flag| (flag.name(), flag.kind.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("anything".to_string(), Kind::Json),
                (
                    "device".to_string(),
                    Kind::Choice(vec![
                        (json!("cpu"), Some("The CPU.".to_string())),
                        (json!("gpu"), Some("The first GPU.".to_string())),
                    ])
                ),
                ("epochs".to_string(), Kind::Integer),
                ("labels".to_string(), Kind::Json),
                ("layers".to_string(), Kind::Json),
                (
                    "optimizer.kind".to_string(),
                    Kind::Choice(vec![(json!("adam"), None), (json!("sgd"), None)])
                ),
                ("optimizer.lr".to_string(), Kind::Float),
                ("resume-from".to_string(), Kind::Json),
                ("seed".to_string(), Kind::Integer),
                ("verbose".to_string(), Kind::Bool),
            ]
        );
        assert_eq!(
            flag(&flags, "epochs").help.as_deref(),
            Some("Passes over the data.")
        );
        assert_eq!(
            flag(&flags, "optimizer.lr").help.as_deref(),
            Some("The learning rate.")
        );
        assert_eq!(
            flag(&flags, "device").help.as_deref(),
            Some("Where to train.")
        );
        assert_eq!(flag(&flags, "seed").help, None);
        assert_eq!(flag(&flags, "verbose").default, Some(json!(false)));
    }

    #[test]
    fn a_field_the_schema_requires_is_required_unless_the_example_gives_it() {
        let without_example = flags(&definition(Some(config_schema()), None));
        let with_example = flags(&definition(
            Some(config_schema()),
            Some(json!({"epochs": 10, "optimizer": {"lr": 0.1}})),
        ));

        let required = |flags: &[Flag]| -> Vec<String> {
            flags
                .iter()
                .filter(|flag| flag.required)
                .map(Flag::name)
                .collect()
        };
        assert_eq!(
            required(&without_example),
            [
                "device",
                "epochs",
                "labels",
                "layers",
                "optimizer.kind",
                "optimizer.lr"
            ]
        );
        assert_eq!(
            required(&with_example),
            ["device", "labels", "layers", "optimizer.kind"]
        );
        assert_eq!(flag(&with_example, "epochs").default, Some(json!(10)));
        assert_eq!(
            flag(&with_example, "optimizer.lr").default,
            Some(json!(0.1))
        );
    }

    #[test]
    fn a_schema_that_is_not_an_object_leaves_the_flags_to_the_example() {
        let flags = flags(&definition(Some(json!(true)), Some(json!({"epochs": 10}))));

        assert_eq!(
            summary(&flags),
            [("epochs".to_string(), Kind::Integer, Some(json!(10)))]
        );
    }

    #[test]
    fn a_schema_that_refers_to_itself_takes_json_where_it_does() {
        let flags = flags(&definition(
            Some(json!({
                "type": "object",
                "properties": {"node": {"$ref": "#"}, "loop": {"$ref": "#/$defs/A"}},
                "$defs": {"A": {"$ref": "#/$defs/B"}, "B": {"$ref": "#/$defs/A"}}
            })),
            None,
        ));

        let kinds: Vec<(String, Kind)> = flags
            .iter()
            .map(|flag| (flag.name(), flag.kind.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("loop".to_string(), Kind::Json),
                ("node.loop".to_string(), Kind::Json),
                ("node.node".to_string(), Kind::Json),
            ]
        );
    }

    #[test]
    fn reserved_unusable_and_taken_names_have_no_flag() {
        let flags = flags(&definition(
            None,
            Some(json!({
                "help": true,
                "version": "1",
                "config": {"path": "a"},
                "optimizer": {"config": 1},
                "a.b": 1,
                "a b": 1,
                "-x": 1,
                "": 1,
                "batch-size": 1,
                "batch_size": 2
            })),
        ));

        assert_eq!(
            summary(&flags),
            [
                ("batch-size".to_string(), Kind::Integer, Some(json!(1))),
                ("config.path".to_string(), Kind::String, Some(json!("a"))),
                (
                    "optimizer.config".to_string(),
                    Kind::Integer,
                    Some(json!(1))
                ),
            ]
        );
    }

    #[test]
    fn arguments_parse_as_the_kind_of_their_field() {
        assert_eq!(Kind::Integer.parse("-3"), Ok(json!(-3)));
        assert!(Kind::Integer.parse("1.5").is_err());
        assert!(Kind::Integer.parse("ten").is_err());
        assert_eq!(Kind::Float.parse("3e-4"), Ok(json!(3e-4)));
        assert_eq!(Kind::Float.parse("2"), Ok(json!(2)));
        assert_eq!(Kind::Float.parse(".5"), Ok(json!(0.5)));
        assert!(Kind::Float.parse("inf").is_err());
        assert_eq!(Kind::Bool.parse("false"), Ok(json!(false)));
        assert!(Kind::Bool.parse("yes").is_err());
        assert_eq!(Kind::String.parse("12"), Ok(json!("12")));
        assert_eq!(Kind::Json.parse("[1, 2]"), Ok(json!([1, 2])));
        assert!(Kind::Json.parse("[1,").is_err());
        let choice = Kind::Choice(vec![(json!("adam"), None), (json!(2), None)]);
        assert_eq!(choice.parse("2"), Ok(json!(2)));
        assert!(choice.parse("sgd").is_err());
    }

    #[test]
    fn defaults_render_as_arguments_their_flag_takes() {
        assert_eq!(
            Kind::Float.render(&json!(5e-5f32)).as_deref(),
            Some("0.00005")
        );
        assert_eq!(Kind::Float.render(&json!(0.001)).as_deref(), Some("0.001"));
        assert_eq!(Kind::Float.render(&json!(10)).as_deref(), Some("10"));
        assert_eq!(Kind::Integer.render(&json!(64)).as_deref(), Some("64"));
        assert_eq!(
            Kind::Json.render(&json!({"a": [1]})).as_deref(),
            Some(r#"{"a":[1]}"#)
        );
        assert_eq!(Kind::Integer.render(&json!(1.5)), None);
        assert_eq!(Kind::String.render(&json!(3)), None);
        assert_eq!(Kind::Float.render(&Value::Null), None);
        let choice = Kind::Choice(vec![(json!("adam"), None)]);
        assert_eq!(choice.render(&json!("adam")).as_deref(), Some("adam"));
        assert_eq!(choice.render(&json!("sgd")), None);
    }
}
