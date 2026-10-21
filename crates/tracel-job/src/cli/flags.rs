//! The flags of a job's command line, read from its input schema: one per field of its input.

use std::error::Error;
use std::ptr;

use referencing::{Registry, Resolver, Retrieve, Uri};
use serde_json::{Map, Number, Value};

use crate::JobDefinition;

/// Flag names the command line keeps: a field with one of these names has no flag.
const RESERVED: [&str; 2] = ["help", "config"];

/// The URI the input schema is registered under, which its references resolve against.
const BASE: &str = "json-schema:///input";

/// A flag that sets one field of a job's input.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// The keys of the field, from the top of the input down.
    pub path: Vec<String>,
    /// The values the flag takes.
    pub kind: Kind,
    /// What the field is, from the schema.
    pub help: Option<String>,
    /// The field's value in the default input, or else its schema default.
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

/// The values a flag takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// An integer.
    Integer,
    /// A number.
    Number,
    /// A string, taken as given.
    String,
    /// One of these values.
    Choice(Vec<Choice>),
    /// A JSON literal.
    Json,
}

/// A value a flag takes, written as its argument, with its description.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The argument that gives the value.
    pub text: String,
    /// The value.
    pub value: Value,
    /// What the value means.
    pub help: Option<String>,
}

impl Choice {
    /// `value`, described by `help`, when it is a string, a number or a boolean.
    fn new(value: &Value, help: Option<String>) -> Option<Self> {
        Some(Self {
            text: scalar_text(value)?,
            value: value.clone(),
            help,
        })
    }
}

impl Kind {
    /// Parses a flag's argument as the field's value.
    pub fn parse(&self, argument: &str) -> Result<Value, String> {
        match self {
            Self::Bool => argument
                .parse()
                .map(Value::Bool)
                .map_err(|_| "expected true or false".to_string()),
            Self::Integer => match number(argument) {
                Some(number) if !number.is_f64() => Ok(Value::Number(number)),
                _ => Err("expected an integer".to_string()),
            },
            Self::Number => number(argument)
                .map(Value::Number)
                .ok_or_else(|| "expected a number".to_string()),
            Self::String => Ok(Value::String(argument.to_string())),
            Self::Choice(choices) => choices
                .iter()
                .find(|choice| choice.text == argument)
                .map(|choice| choice.value.clone())
                .ok_or_else(|| "not one of the possible values".to_string()),
            Self::Json => {
                serde_json::from_str(argument).map_err(|error| format!("not JSON: {error}"))
            }
        }
    }

    /// `value` written as the flag's argument, when the flag takes it.
    pub fn render(&self, value: &Value) -> Option<String> {
        let text = match (self, value) {
            (Self::Json, value) => value.to_string(),
            (Self::Number, Value::Number(number)) => float_text(number.as_f64()?),
            (Self::String, Value::String(text)) => text.clone(),
            (Self::String, _) => return None,
            (_, value) => scalar_text(value)?,
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
fn scalar_text(value: &Value) -> Option<String> {
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

/// The flags of `job`, one per field of its input, when its input schema describes an object;
/// none otherwise.
///
/// A field whose key is not made of letters, digits, `_` and `-`, whose flag name is reserved, or
/// whose flag name an earlier field took, has no flag.
pub fn flags(job: &JobDefinition) -> Vec<Flag> {
    let Some(schema) = &job.input_schema else {
        return Vec::new();
    };
    let Ok(registry) = Registry::new()
        .retriever(Unfetched)
        .add(BASE, schema)
        .and_then(|registry| registry.prepare())
    else {
        return Vec::new();
    };
    let base = referencing::uri::from_str(BASE).expect("the base is a URI");
    let mut walk = Walk {
        resolver: registry.resolver(base),
        entered: Vec::new(),
        flags: Vec::new(),
    };
    if let Shape::Object(object) = walk.shape(schema) {
        walk.object(object, &mut Vec::new(), job.input_example.as_ref(), true);
    }

    let mut names = Vec::new();
    walk.flags.retain(|flag| {
        let name = flag.name();
        let keep = !RESERVED.contains(&name.as_str()) && !names.contains(&name);
        names.push(name);
        keep
    });
    walk.flags
}

/// Answers each schema the input schema refers to outside itself with `true`, the schema of any
/// value, since none is fetched: the registry is prepared whatever the input schema refers to, and
/// a reference into such a schema does not resolve.
struct Unfetched;

impl Retrieve for Unfetched {
    fn retrieve(&self, _: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        Ok(Value::Bool(true))
    }
}

/// Whether `key` can be part of a flag name.
fn is_flag_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('-')
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// What a schema describes: an object whose fields are walked into, or the values of one flag.
enum Shape<'r> {
    Object(&'r Map<String, Value>),
    Leaf(Kind),
}

/// Walks an input schema, adding a flag for each field.
struct Walk<'r> {
    /// Resolves the schema's references, against its root, where schemars puts its definitions.
    resolver: Resolver<'r>,
    /// The objects the walk is in, which a field that refers back to one, as a recursive type
    /// does, does not enter again.
    entered: Vec<&'r Map<String, Value>>,
    flags: Vec<Flag>,
}

impl<'r> Walk<'r> {
    /// The schema `schema` refers to with `$ref`, followed until one does not refer on, or
    /// `schema` itself; `None` when a reference does not resolve or comes back round.
    fn target(&self, mut schema: &'r Value) -> Option<&'r Value> {
        let mut followed = Vec::new();
        while let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if followed.contains(&reference) {
                return None;
            }
            followed.push(reference);
            schema = self.resolver.lookup(reference).ok()?.contents();
        }
        Some(schema)
    }

    /// What `schema` describes. An object the walk is in, or a nullable one, takes a JSON
    /// literal.
    fn shape(&self, schema: &'r Value) -> Shape<'r> {
        let Some(schema) = self.target(schema).and_then(Value::as_object) else {
            return Shape::Leaf(Kind::Json);
        };
        let (types, nullable) = types(schema);
        let has_properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| !properties.is_empty());
        if has_properties && matches!(types[..], [] | ["object"]) {
            let entered = self.entered.iter().any(|entered| ptr::eq(*entered, schema));
            return if nullable || entered {
                Shape::Leaf(Kind::Json)
            } else {
                Shape::Object(schema)
            };
        }
        if let Some(choices) = self.choices(schema).filter(|choices| !choices.is_empty()) {
            return Shape::Leaf(Kind::Choice(choices));
        }
        if let Some(value) = self.nullable(schema) {
            return match self.shape(value) {
                Shape::Object(_) => Shape::Leaf(Kind::Json),
                leaf => leaf,
            };
        }
        Shape::Leaf(match types[..] {
            ["boolean"] => Kind::Bool,
            ["integer"] => Kind::Integer,
            ["number"] => Kind::Number,
            ["string"] => Kind::String,
            _ => Kind::Json,
        })
    }

    /// The values `schema` lists, with `const`, `enum`, or `oneOf` or `anyOf` of schemas that
    /// list values, `null` left out; `None` when it lists none, or one that is not a string, a
    /// number or a boolean.
    fn choices(&self, schema: &'r Map<String, Value>) -> Option<Vec<Choice>> {
        if let Some(value) = schema.get("const") {
            return Some(vec![Choice::new(value, description(schema))?]);
        }
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            return values
                .iter()
                .filter(|value| !value.is_null())
                .map(|value| Choice::new(value, None))
                .collect();
        }
        let mut choices = Vec::new();
        for variant in variants(schema)? {
            let variant = self.target(variant)?.as_object()?;
            if !is_null(variant) {
                choices.extend(self.choices(variant)?);
            }
        }
        Some(choices)
    }

    /// The schema of the value of a `oneOf` or `anyOf` of one value and `null`.
    fn nullable(&self, schema: &'r Map<String, Value>) -> Option<&'r Value> {
        let [first, second] = variants(schema)?.as_slice() else {
            return None;
        };
        let is_null = |variant| {
            self.target(variant)
                .and_then(Value::as_object)
                .is_some_and(is_null)
        };
        match (is_null(first), is_null(second)) {
            (false, true) => Some(first),
            (true, false) => Some(second),
            _ => None,
        }
    }

    /// The description of `schema`, or else that of the schema it refers to, or else the help of
    /// the value it makes optional.
    fn help(&self, schema: &'r Value) -> Option<String> {
        schema.as_object().and_then(description).or_else(|| {
            let target = self.target(schema)?.as_object()?;
            description(target).or_else(|| self.help(self.nullable(target)?))
        })
    }

    /// Adds the flags of the fields of `object`, the schema of the object at `path`, whose value
    /// in the default input is `defaults`. The object is required when `required` is. A field's
    /// help is its [`help`](Self::help).
    fn object(
        &mut self,
        object: &'r Map<String, Value>,
        path: &mut Vec<String>,
        defaults: Option<&'r Value>,
        required: bool,
    ) {
        let Some(properties) = object.get("properties").and_then(Value::as_object) else {
            return;
        };
        let required_keys: Vec<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        self.entered.push(object);
        for (key, property) in properties {
            if !is_flag_key(key) {
                continue;
            }
            path.push(key.clone());
            let default = defaults
                .and_then(|defaults| defaults.get(key))
                .or_else(|| property.get("default"));
            let required = required && required_keys.contains(&key.as_str());
            match self.shape(property) {
                Shape::Object(object) => self.object(object, path, default, required),
                Shape::Leaf(kind) => {
                    self.flags.push(Flag {
                        path: path.clone(),
                        kind,
                        help: self.help(property),
                        default: default.cloned(),
                        required: required && default.is_none(),
                    });
                }
            }
            path.pop();
        }
        self.entered.pop();
    }
}

/// The types `schema` gives, but `null`, and whether it gives `null`.
fn types(schema: &Map<String, Value>) -> (Vec<&str>, bool) {
    let names: Vec<&str> = match schema.get("type") {
        Some(Value::String(name)) => vec![name],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let nullable = names.contains(&"null");
    let types = names.into_iter().filter(|name| *name != "null").collect();
    (types, nullable)
}

/// The schemas of `schema`'s `oneOf`, or else `anyOf`.
fn variants(schema: &Map<String, Value>) -> Option<&Vec<Value>> {
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
}

/// Whether `schema` describes only `null`.
fn is_null(schema: &Map<String, Value>) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
}

/// The description of what `schema` describes.
fn description(schema: &Map<String, Value>) -> Option<String> {
    schema
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rstest::{fixture, rstest};
    use schemars::JsonSchema;
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    fn flags_of(schema: Value, default: Option<Value>) -> Vec<Flag> {
        flags(&JobDefinition {
            name: "train".to_string(),
            description: None,
            input_schema: Some(schema),
            input_example: default,
        })
    }

    fn flags_for<T: JsonSchema>() -> Vec<Flag> {
        flags_of(schemars::schema_for!(T).to_value(), None)
    }

    fn flag<'f>(flags: &'f [Flag], name: &str) -> &'f Flag {
        flags
            .iter()
            .find(|flag| flag.name() == name)
            .unwrap_or_else(|| panic!("no flag --{name}"))
    }

    fn kind_of<T: JsonSchema>() -> Kind {
        flag(&flags_for::<Field<T>>(), "field").kind.clone()
    }

    fn one_of(values: &[Value]) -> Kind {
        Kind::Choice(
            values
                .iter()
                .map(|value| Choice::new(value, None).unwrap())
                .collect(),
        )
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Field<T> {
        field: T,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Nesting {
        optimizer: Nested,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Nested {
        lr: f64,
        weight_decay: f64,
        schedule: Schedule,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Schedule {
        warmup: u32,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Checkpoint {
        experiment: u32,
    }

    #[derive(JsonSchema)]
    #[serde(tag = "kind")]
    #[expect(dead_code)]
    enum Tagged {
        Adam { beta: f64 },
        Sgd,
    }

    #[derive(JsonSchema)]
    #[serde(untagged)]
    #[expect(dead_code)]
    enum Untagged {
        Count(u32),
        Name(String),
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    enum Mixed {
        Unit,
        Data(u32),
    }

    #[rstest]
    #[case::unsigned_integer(kind_of::<u32>(), Kind::Integer)]
    #[case::signed_integer(kind_of::<i64>(), Kind::Integer)]
    #[case::double(kind_of::<f64>(), Kind::Number)]
    #[case::single(kind_of::<f32>(), Kind::Number)]
    #[case::boolean(kind_of::<bool>(), Kind::Bool)]
    #[case::string(kind_of::<String>(), Kind::String)]
    #[case::character(kind_of::<char>(), Kind::String)]
    fn a_scalar_field_takes_the_values_of_its_type(#[case] kind: Kind, #[case] expected: Kind) {
        assert_eq!(kind, expected);
    }

    #[rstest]
    #[case::integer(kind_of::<Option<u64>>(), kind_of::<u64>())]
    #[case::string(kind_of::<Option<String>>(), kind_of::<String>())]
    #[case::unit_enum(kind_of::<Option<Backend>>(), kind_of::<Backend>())]
    fn an_optional_field_takes_the_values_of_the_type_it_makes_optional(
        #[case] optional: Kind,
        #[case] plain: Kind,
    ) {
        assert_eq!(optional, plain);
    }

    #[rstest]
    #[case::list(kind_of::<Vec<u32>>())]
    #[case::map(kind_of::<HashMap<String, String>>())]
    #[case::tuple(kind_of::<(u32, String)>())]
    #[case::any_json(kind_of::<Value>())]
    #[case::unit(kind_of::<()>())]
    #[case::optional_object(kind_of::<Option<Checkpoint>>())]
    #[case::tagged_enum(kind_of::<Tagged>())]
    #[case::untagged_enum(kind_of::<Untagged>())]
    #[case::enum_with_data(kind_of::<Mixed>())]
    fn a_field_of_any_other_type_takes_json(#[case] kind: Kind) {
        assert_eq!(kind, Kind::Json);
    }

    #[test]
    fn a_nested_object_has_a_flag_per_field_named_by_its_keys() {
        let mut names: Vec<String> = flags_for::<Nesting>().iter().map(Flag::name).collect();
        names.sort();

        assert_eq!(
            names,
            [
                "optimizer.lr",
                "optimizer.schedule.warmup",
                "optimizer.weight-decay"
            ]
        );
    }

    #[test]
    fn an_object_that_may_be_null_takes_json() {
        let schema = json!({
            "type": "object",
            "properties": {
                "resume": {
                    "type": ["object", "null"],
                    "properties": {"epoch": {"type": "integer"}}
                }
            }
        });

        assert_eq!(flag(&flags_of(schema, None), "resume").kind, Kind::Json);
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Documented {
        #[schemars(description = "Passes over the data.")]
        epochs: u32,
        backend: Backend,
        #[schemars(description = "The backend to compare with.")]
        baseline: Backend,
        fallback: Option<Backend>,
        seed: u64,
    }

    #[derive(JsonSchema)]
    #[schemars(description = "The tensor backend.")]
    #[expect(dead_code)]
    enum Backend {
        Wgpu,
        Cuda,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    enum Device {
        #[schemars(description = "The CPU.")]
        Cpu,
        #[schemars(description = "The first GPU.")]
        Gpu,
    }

    #[rstest]
    #[case::its_own_description("epochs", Some("Passes over the data."))]
    #[case::the_description_of_its_type("backend", Some("The tensor backend."))]
    #[case::its_own_over_its_types("baseline", Some("The backend to compare with."))]
    #[case::that_of_the_type_it_makes_optional("fallback", Some("The tensor backend."))]
    #[case::none_when_nothing_describes_it("seed", None)]
    fn a_flag_is_helped_by_its_fields_description_or_else_its_types(
        #[case] name: &str,
        #[case] help: Option<&str>,
    ) {
        assert_eq!(flag(&flags_for::<Documented>(), name).help.as_deref(), help);
    }

    #[rstest]
    #[case::described(
        kind_of::<Device>(),
        &[("Cpu", Some("The CPU.")), ("Gpu", Some("The first GPU."))]
    )]
    #[case::undescribed(kind_of::<Backend>(), &[("Wgpu", None), ("Cuda", None)])]
    fn the_unit_variants_of_an_enum_are_choices_with_their_descriptions(
        #[case] kind: Kind,
        #[case] expected: &[(&str, Option<&str>)],
    ) {
        let Kind::Choice(choices) = &kind else {
            panic!("{kind:?} is not a choice");
        };
        let choices: Vec<(&str, Option<&str>)> = choices
            .iter()
            .map(|choice| (choice.text.as_str(), choice.help.as_deref()))
            .collect();

        assert_eq!(choices, expected);
    }

    #[rstest]
    #[case::leaving_out_null(json!(["adam", null]), &[json!("adam")])]
    #[case::keeping_numbers_as_numbers(json!([1, 2.5]), &[json!(1), json!(2.5)])]
    fn listed_values_are_the_choices_of_their_flag(
        #[case] listed: Value,
        #[case] values: &[Value],
    ) {
        let schema = json!({"type": "object", "properties": {"field": {"enum": listed}}});

        assert_eq!(flag(&flags_of(schema, None), "field").kind, one_of(values));
    }

    #[test]
    fn listed_values_of_which_one_is_not_a_scalar_take_json() {
        let schema = json!({"type": "object", "properties": {"field": {"enum": ["adam", [1]]}}});

        assert_eq!(flag(&flags_of(schema, None), "field").kind, Kind::Json);
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Defaulted {
        epochs: u32,
        #[serde(default = "seed")]
        seed: u64,
        #[serde(default)]
        schedule: Scheduled,
    }

    #[derive(Serialize, JsonSchema)]
    #[serde(default)]
    struct Scheduled {
        warmup: u32,
        cooldown: u32,
    }

    impl Default for Scheduled {
        fn default() -> Self {
            Self {
                warmup: 5,
                cooldown: 2,
            }
        }
    }

    fn seed() -> u64 {
        42
    }

    #[rstest]
    #[case::without_a_schema_default("epochs")]
    #[case::over_the_schema_default("seed")]
    fn a_flag_defaults_to_its_fields_value_in_the_default_input(#[case] name: &str) {
        let default_input = json!({"epochs": 10, "seed": 7});

        let flags = flags_of(
            schemars::schema_for!(Defaulted).to_value(),
            Some(default_input.clone()),
        );

        assert_eq!(flag(&flags, name).default.as_ref(), default_input.get(name));
    }

    #[rstest]
    #[case::of_a_field("seed", None, json!(seed()))]
    #[case::of_a_nested_field("schedule.warmup", None, json!(Scheduled::default().warmup))]
    #[case::of_a_nested_field_the_default_input_leaves_out(
        "schedule.cooldown",
        Some(json!({"schedule": {"warmup": 1}})),
        json!(Scheduled::default().cooldown)
    )]
    fn a_flag_the_default_input_does_not_give_defaults_to_the_schema_default(
        #[case] name: &str,
        #[case] default_input: Option<Value>,
        #[case] default: Value,
    ) {
        let flags = flags_of(schemars::schema_for!(Defaulted).to_value(), default_input);

        assert_eq!(flag(&flags, name).default, Some(default));
    }

    #[fixture]
    fn requirements() -> Value {
        json!({
            "type": "object",
            "required": ["epochs", "seed", "optimizer"],
            "properties": {
                "epochs": {"type": "integer"},
                "seed": {"type": "integer", "default": 42},
                "tag": {"type": "string"},
                "optimizer": {
                    "type": "object",
                    "required": ["lr"],
                    "properties": {"lr": {"type": "number"}}
                },
                "resume": {
                    "type": "object",
                    "required": ["epoch"],
                    "properties": {"epoch": {"type": "integer"}}
                }
            }
        })
    }

    #[rstest]
    #[case::required("epochs", true)]
    #[case::required_in_a_required_object("optimizer.lr", true)]
    #[case::required_with_a_default("seed", false)]
    #[case::optional("tag", false)]
    #[case::required_in_an_optional_object("resume.epoch", false)]
    fn a_flag_is_required_when_the_schema_requires_its_field_and_gives_no_default(
        requirements: Value,
        #[case] name: &str,
        #[case] required: bool,
    ) {
        assert_eq!(flag(&flags_of(requirements, None), name).required, required);
    }

    #[rstest]
    fn a_required_field_the_default_input_gives_is_not_a_required_flag(requirements: Value) {
        let flags = flags_of(requirements, Some(json!({"epochs": 10})));

        assert!(!flag(&flags, "epochs").required);
    }

    #[test]
    fn a_job_without_an_input_schema_has_no_flags_whatever_its_default_input() {
        let job = JobDefinition {
            name: "train".to_string(),
            description: None,
            input_schema: None,
            input_example: Some(json!({"epochs": 10})),
        };

        assert!(flags(&job).is_empty());
    }

    #[rstest]
    #[case::of_an_integer(schemars::schema_for!(u32).to_value())]
    #[case::of_a_list(schemars::schema_for!(Vec<u32>).to_value())]
    #[case::of_a_unit_enum(schemars::schema_for!(Backend).to_value())]
    #[case::of_an_optional_object(schemars::schema_for!(Option<Checkpoint>).to_value())]
    #[case::of_any_json(schemars::schema_for!(Value).to_value())]
    #[case::of_an_object_without_fields(json!({"type": "object", "properties": {}}))]
    fn an_input_that_is_not_an_object_with_fields_has_no_flags(#[case] schema: Value) {
        assert!(flags_of(schema, None).is_empty());
    }

    #[test]
    fn a_field_that_refers_back_to_an_object_it_is_in_takes_json() {
        let schema = json!({"type": "object", "properties": {"node": {"$ref": "#"}}});

        assert_eq!(flag(&flags_of(schema, None), "node").kind, Kind::Json);
    }

    #[rstest]
    #[case::missing_from_the_schema("#/$defs/Missing")]
    #[case::that_comes_back_round("#/$defs/Loop")]
    #[case::into_another_document("https://example.com/input.json#/$defs/Epochs")]
    fn a_field_whose_reference_does_not_resolve_takes_json(#[case] reference: &str) {
        let schema = json!({
            "type": "object",
            "properties": {"field": {"$ref": reference}},
            "$defs": {"Loop": {"$ref": "#/$defs/Loop"}}
        });

        assert_eq!(flag(&flags_of(schema, None), "field").kind, Kind::Json);
    }

    #[rstest]
    #[case::from_a_field(json!({
        "type": "object",
        "properties": {
            "epochs": {"type": "integer"},
            "remote": {"$ref": "https://example.com/input.json"}
        }
    }))]
    #[case::from_a_definition_to_a_file_beside_it(json!({
        "type": "object",
        "properties": {"epochs": {"type": "integer"}},
        "$defs": {"Shared": {"$ref": "shared.json"}}
    }))]
    #[case::to_its_dialect(json!({
        "$schema": "https://example.com/an-unknown-dialect",
        "type": "object",
        "properties": {"epochs": {"type": "integer"}}
    }))]
    fn a_schema_that_refers_outside_itself_keeps_its_flags(#[case] schema: Value) {
        assert_eq!(flag(&flags_of(schema, None), "epochs").kind, Kind::Integer);
    }

    #[rstest]
    #[case::into_defs("$defs")]
    #[case::into_definitions("definitions")]
    fn a_field_takes_the_values_of_the_schema_its_reference_resolves_to(#[case] definitions: &str) {
        let schema = json!({
            "type": "object",
            "properties": {"epochs": {"$ref": format!("#/{definitions}/Epochs")}},
            definitions: {"Epochs": {"type": "integer"}}
        });

        assert_eq!(flag(&flags_of(schema, None), "epochs").kind, Kind::Integer);
    }

    #[rstest]
    #[case::reserved_for_help("help")]
    #[case::reserved_for_the_config_file("config")]
    #[case::with_a_dot("a.b")]
    #[case::with_a_space("a b")]
    #[case::starting_with_a_dash("-x")]
    #[case::empty("")]
    fn a_field_whose_key_cannot_name_a_flag_has_none(#[case] key: &str) {
        let schema = json!({"type": "object", "properties": {key: {"type": "integer"}}});

        assert!(flags_of(schema, None).is_empty());
    }

    #[rstest]
    #[case::as_the_object("config", "path")]
    #[case::as_the_field("optimizer", "config")]
    fn a_reserved_name_is_reserved_only_as_a_whole_flag_name(
        #[case] object: &str,
        #[case] field: &str,
    ) {
        let schema = json!({
            "type": "object",
            "properties": {object: {"type": "object", "properties": {field: {"type": "integer"}}}}
        });

        let names: Vec<String> = flags_of(schema, None).iter().map(Flag::name).collect();

        assert_eq!(names, [format!("{object}.{field}")]);
    }

    #[test]
    fn a_flag_name_an_earlier_field_took_is_not_given_again() {
        let schema = json!({
            "type": "object",
            "properties": {"batch-size": {"type": "integer"}, "batch_size": {"type": "integer"}}
        });

        assert_eq!(flags_of(schema, None).len(), 1);
    }

    #[rstest]
    #[case::a_negative_integer(Kind::Integer, "-3", json!(-3))]
    #[case::a_number_in_exponent_form(Kind::Number, "3e-4", json!(3e-4))]
    #[case::a_whole_number_kept_whole(Kind::Number, "2", json!(2))]
    #[case::a_number_as_rust_reads_it(Kind::Number, ".5", json!(0.5))]
    #[case::a_bool(Kind::Bool, "false", json!(false))]
    #[case::digits_as_a_string(Kind::String, "12", json!("12"))]
    #[case::json(Kind::Json, "[1, 2]", json!([1, 2]))]
    #[case::a_choice_as_its_value(one_of(&[json!("adam"), json!(2)]), "2", json!(2))]
    fn an_argument_parses_as_the_value_its_kind_reads(
        #[case] kind: Kind,
        #[case] argument: &str,
        #[case] value: Value,
    ) {
        assert_eq!(kind.parse(argument), Ok(value));
    }

    #[rstest]
    #[case::a_fraction_as_an_integer(Kind::Integer, "1.5")]
    #[case::a_word_as_an_integer(Kind::Integer, "ten")]
    #[case::infinity_as_a_number(Kind::Number, "inf")]
    #[case::yes_as_a_bool(Kind::Bool, "yes")]
    #[case::json_cut_short(Kind::Json, "[1,")]
    #[case::a_value_no_choice_gives(one_of(&[json!("adam")]), "sgd")]
    fn an_argument_its_kind_does_not_read_is_rejected(#[case] kind: Kind, #[case] argument: &str) {
        assert!(kind.parse(argument).is_err());
    }

    #[rstest]
    #[case::a_number(Kind::Number, json!(0.001))]
    #[case::a_whole_number(Kind::Number, json!(10))]
    #[case::an_integer(Kind::Integer, json!(64))]
    #[case::a_string(Kind::String, json!("baseline"))]
    #[case::a_bool(Kind::Bool, json!(true))]
    #[case::json(Kind::Json, json!({"a": [1]}))]
    #[case::a_choice(one_of(&[json!("adam")]), json!("adam"))]
    fn a_default_renders_as_an_argument_that_parses_back_to_it(
        #[case] kind: Kind,
        #[case] default: Value,
    ) {
        let argument = kind.render(&default).unwrap();

        assert_eq!(kind.parse(&argument), Ok(default));
    }

    #[test]
    fn a_default_widened_from_an_f32_renders_as_the_f32() {
        let single = 5e-5f32;

        assert_eq!(
            Kind::Number.render(&json!(single)),
            Some(single.to_string())
        );
    }

    #[rstest]
    #[case::a_fraction_as_an_integer(Kind::Integer, json!(1.5))]
    #[case::a_number_as_a_string(Kind::String, json!(3))]
    #[case::null_as_a_number(Kind::Number, Value::Null)]
    #[case::a_value_no_choice_gives(one_of(&[json!("adam")]), json!("sgd"))]
    fn a_default_its_flag_does_not_take_renders_as_nothing(
        #[case] kind: Kind,
        #[case] default: Value,
    ) {
        assert_eq!(kind.render(&default), None);
    }
}
