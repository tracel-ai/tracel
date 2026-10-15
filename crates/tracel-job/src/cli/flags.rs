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
#[allow(dead_code)]
mod tests {
    use std::collections::HashMap;

    use rstest::rstest;
    use schemars::JsonSchema;
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    /// The flags of a job whose input schema is `schema` and default input `default`.
    fn flags_of(schema: Value, default: Option<Value>) -> Vec<Flag> {
        flags(&JobDefinition {
            name: "train".to_string(),
            description: None,
            input_schema: Some(schema),
            input_example: default,
        })
    }

    /// The flags of a job taking `T`, with no default input.
    fn flags_for<T: JsonSchema>() -> Vec<Flag> {
        flags_of(schemars::schema_for!(T).to_value(), None)
    }

    /// The name and kind of each flag.
    fn kinds(flags: &[Flag]) -> Vec<(String, Kind)> {
        flags
            .iter()
            .map(|flag| (flag.name(), flag.kind.clone()))
            .collect()
    }

    fn flag<'f>(flags: &'f [Flag], name: &str) -> &'f Flag {
        flags
            .iter()
            .find(|flag| flag.name() == name)
            .unwrap_or_else(|| panic!("no flag --{name} in {:?}", kinds(flags)))
    }

    fn choices(values: &[(&str, Option<&str>)]) -> Kind {
        Kind::Choice(
            values
                .iter()
                .map(|(value, help)| Choice {
                    text: value.to_string(),
                    value: json!(value),
                    help: help.map(str::to_string),
                })
                .collect(),
        )
    }

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    struct Shapes {
        epochs: u32,
        offset: i64,
        lr: f64,
        decay: f32,
        shuffle: bool,
        tag: String,
        letter: char,
        #[serde(rename = "seed_value")]
        seed: Option<u64>,
        label: Option<String>,
        optimizer: Optimizer,
        resume_from: Option<Resume>,
        layers: Vec<u32>,
        labels: HashMap<String, String>,
        pair: (u32, String),
        anything: serde_json::Value,
        nothing: (),
        tagged: Tagged,
        untagged: Untagged,
        mixed: Mixed,
        #[serde(flatten)]
        flattened: Flattened,
    }

    #[derive(Serialize, JsonSchema)]
    struct Optimizer {
        lr: f64,
        weight_decay: f64,
        schedule: Schedule,
    }

    #[derive(Serialize, JsonSchema)]
    struct Schedule {
        warmup: u32,
    }

    #[derive(JsonSchema)]
    struct Resume {
        experiment: u32,
    }

    #[derive(JsonSchema)]
    #[serde(tag = "kind")]
    enum Tagged {
        Adam { beta: f64 },
        Sgd,
    }

    #[derive(JsonSchema)]
    #[serde(untagged)]
    enum Untagged {
        Count(u32),
        Name(String),
    }

    #[derive(JsonSchema)]
    enum Mixed {
        Unit,
        Data(u32),
    }

    #[derive(JsonSchema)]
    struct Flattened {
        workers: u32,
    }

    #[test]
    fn objects_nest_into_dotted_flags_and_other_values_take_json() {
        let flags = flags_for::<Shapes>();

        assert_eq!(
            kinds(&flags),
            [
                ("anything".to_string(), Kind::Json),
                ("decay".to_string(), Kind::Number),
                ("epochs".to_string(), Kind::Integer),
                ("label".to_string(), Kind::String),
                ("labels".to_string(), Kind::Json),
                ("layers".to_string(), Kind::Json),
                ("letter".to_string(), Kind::String),
                ("lr".to_string(), Kind::Number),
                ("mixed".to_string(), Kind::Json),
                ("nothing".to_string(), Kind::Json),
                ("offset".to_string(), Kind::Integer),
                ("optimizer.lr".to_string(), Kind::Number),
                ("optimizer.schedule.warmup".to_string(), Kind::Integer),
                ("optimizer.weight-decay".to_string(), Kind::Number),
                ("pair".to_string(), Kind::Json),
                ("resumeFrom".to_string(), Kind::Json),
                ("seed-value".to_string(), Kind::Integer),
                ("shuffle".to_string(), Kind::Bool),
                ("tag".to_string(), Kind::String),
                ("tagged".to_string(), Kind::Json),
                ("untagged".to_string(), Kind::Json),
                ("workers".to_string(), Kind::Integer),
            ]
        );
        let weight_decay = flag(&flags, "optimizer.weight-decay");
        assert_eq!(weight_decay.path, ["optimizer", "weight_decay"]);
        assert_eq!(weight_decay.id(), "optimizer.weight_decay");
    }

    /// How to train.
    #[derive(JsonSchema)]
    struct Documented {
        /// Passes over the data.
        epochs: u32,
        /// Where to train.
        device: Device,
        backend: Backend,
        fallback: Option<Backend>,
        /// The backend to compare with.
        baseline: Option<Backend>,
        optimizer: Optimizer,
    }

    #[derive(JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum Device {
        /// The CPU.
        Cpu,
        /// The first GPU.
        Gpu,
    }

    /// The tensor backend.
    #[derive(JsonSchema)]
    enum Backend {
        Wgpu,
        Cuda,
    }

    #[test]
    fn doc_comments_are_help_and_unit_variants_are_choices() {
        let flags = flags_for::<Documented>();

        assert_eq!(
            flag(&flags, "device").kind,
            choices(&[("cpu", Some("The CPU.")), ("gpu", Some("The first GPU."))])
        );
        assert_eq!(
            flag(&flags, "backend").kind,
            choices(&[("Wgpu", None), ("Cuda", None)])
        );
        assert_eq!(flag(&flags, "fallback").kind, flag(&flags, "backend").kind);
        for (name, help) in [
            ("epochs", Some("Passes over the data.")),
            ("device", Some("Where to train.")),
            ("backend", Some("The tensor backend.")),
            ("fallback", Some("The tensor backend.")),
            ("baseline", Some("The backend to compare with.")),
            ("optimizer.lr", None),
        ] {
            assert_eq!(flag(&flags, name).help.as_deref(), help, "--{name}");
        }
    }

    #[derive(Serialize, JsonSchema)]
    struct Defaulted {
        epochs: u32,
        #[serde(default)]
        workers: u32,
        #[serde(default = "seed")]
        seed: u64,
        #[serde(default)]
        schedule: Scheduled,
        optimizer: Optimizer,
        tag: Option<String>,
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

    #[test]
    fn defaults_come_from_the_default_input_or_else_the_schema() {
        let schema = schemars::schema_for!(Defaulted).to_value();
        let default = |flags: &[Flag], name| flag(flags, name).default.clone();

        let without = flags_of(schema.clone(), None);
        let with = flags_of(
            schema,
            Some(json!({"epochs": 10, "seed": 7, "schedule": {"warmup": 1}, "optimizer": {}})),
        );

        assert_eq!(default(&without, "epochs"), None);
        assert_eq!(default(&without, "workers"), Some(json!(0)));
        assert_eq!(default(&without, "seed"), Some(json!(42)));
        assert_eq!(default(&without, "schedule.warmup"), Some(json!(5)));
        assert_eq!(default(&without, "schedule.cooldown"), Some(json!(2)));
        assert_eq!(default(&with, "epochs"), Some(json!(10)));
        assert_eq!(default(&with, "seed"), Some(json!(7)));
        assert_eq!(default(&with, "schedule.warmup"), Some(json!(1)));
        assert_eq!(default(&with, "schedule.cooldown"), Some(json!(2)));
    }

    #[test]
    fn a_field_the_schema_requires_is_required_unless_a_default_gives_it() {
        let schema = schemars::schema_for!(Defaulted).to_value();
        let required = |flags: Vec<Flag>| -> Vec<String> {
            flags
                .iter()
                .filter(|flag| flag.required)
                .map(Flag::name)
                .collect()
        };

        assert_eq!(
            required(flags_of(schema.clone(), None)),
            [
                "epochs",
                "optimizer.lr",
                "optimizer.schedule.warmup",
                "optimizer.weight-decay"
            ]
        );
        assert_eq!(
            required(flags_of(
                schema,
                Some(json!({"epochs": 10, "optimizer": {"lr": 0.1, "schedule": {"warmup": 1}}}))
            )),
            ["optimizer.weight-decay"]
        );
    }

    #[derive(JsonSchema)]
    struct Node {
        value: u32,
        next: Option<Box<Node>>,
        children: Vec<Node>,
    }

    #[test]
    fn an_input_that_is_not_an_object_with_fields_has_no_flags() {
        assert!(flags_for::<u32>().is_empty());
        assert!(flags_for::<()>().is_empty());
        assert!(flags_for::<Vec<u32>>().is_empty());
        assert!(flags_for::<Device>().is_empty());
        assert!(flags_for::<Option<Resume>>().is_empty());
        assert!(flags_for::<serde_json::Value>().is_empty());
        assert!(flags_of(json!({"type": "object", "properties": {}}), None).is_empty());
        assert!(
            flags(&JobDefinition {
                name: "train".to_string(),
                description: None,
                input_schema: None,
                input_example: Some(json!({"epochs": 10})),
            })
            .is_empty()
        );
    }

    #[test]
    fn a_field_that_refers_back_to_an_object_it_is_in_takes_json() {
        assert_eq!(
            kinds(&flags_for::<Node>()),
            [
                ("children".to_string(), Kind::Json),
                ("next".to_string(), Kind::Json),
                ("value".to_string(), Kind::Integer),
            ]
        );
        assert_eq!(
            kinds(&flags_of(
                json!({"type": "object", "properties": {"node": {"$ref": "#"}}}),
                None
            )),
            [("node".to_string(), Kind::Json)]
        );
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

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Escaped {
        odd: Odd,
        slash: Slash,
    }

    #[derive(JsonSchema)]
    #[schemars(rename = "Odd name%")]
    #[expect(dead_code)]
    struct Odd {
        a: u32,
    }

    #[derive(JsonSchema)]
    #[schemars(rename = "a/b~c")]
    #[expect(dead_code)]
    struct Slash {
        b: bool,
    }

    #[test]
    fn references_resolve_as_schemars_escapes_them() {
        let schema = schemars::schema_for!(Escaped).to_value();
        assert_eq!(schema["properties"]["odd"]["$ref"], "#/$defs/Odd%20name%25");
        assert_eq!(schema["properties"]["slash"]["$ref"], "#/$defs/a~1b~0c");

        assert_eq!(
            kinds(&flags_of(schema, None)),
            [
                ("odd.a".to_string(), Kind::Integer),
                ("slash.b".to_string(), Kind::Bool),
            ]
        );
    }

    #[test]
    fn reserved_unusable_and_taken_names_have_no_flag() {
        let integer = json!({"type": "integer"});
        let flags = flags_of(
            json!({
                "type": "object",
                "properties": {
                    "help": {"type": "boolean"},
                    "version": {"type": "string"},
                    "config": {"type": "object", "properties": {"path": {"type": "string"}}},
                    "optimizer": {"type": "object", "properties": {"config": integer}},
                    "a.b": integer,
                    "a b": integer,
                    "-x": integer,
                    "": integer,
                    "batch-size": integer,
                    "batch_size": integer
                }
            }),
            None,
        );

        assert_eq!(
            kinds(&flags),
            [
                ("batch-size".to_string(), Kind::Integer),
                ("config.path".to_string(), Kind::String),
                ("optimizer.config".to_string(), Kind::Integer),
                ("version".to_string(), Kind::String),
            ]
        );
    }

    #[test]
    fn arguments_parse_as_the_kind_of_their_field() {
        assert_eq!(Kind::Integer.parse("-3"), Ok(json!(-3)));
        assert!(Kind::Integer.parse("1.5").is_err());
        assert!(Kind::Integer.parse("ten").is_err());
        assert_eq!(Kind::Number.parse("3e-4"), Ok(json!(3e-4)));
        assert_eq!(Kind::Number.parse("2"), Ok(json!(2)));
        assert_eq!(Kind::Number.parse(".5"), Ok(json!(0.5)));
        assert!(Kind::Number.parse("inf").is_err());
        assert_eq!(Kind::Bool.parse("false"), Ok(json!(false)));
        assert!(Kind::Bool.parse("yes").is_err());
        assert_eq!(Kind::String.parse("12"), Ok(json!("12")));
        assert_eq!(Kind::Json.parse("[1, 2]"), Ok(json!([1, 2])));
        assert!(Kind::Json.parse("[1,").is_err());
        let choice = Kind::Choice(vec![
            Choice::new(&json!("adam"), None).unwrap(),
            Choice::new(&json!(2), None).unwrap(),
        ]);
        assert_eq!(choice.parse("2"), Ok(json!(2)));
        assert!(choice.parse("sgd").is_err());
    }

    #[test]
    fn defaults_render_as_arguments_their_flag_takes() {
        assert_eq!(
            Kind::Number.render(&json!(5e-5f32)).as_deref(),
            Some("0.00005")
        );
        assert_eq!(Kind::Number.render(&json!(0.001)).as_deref(), Some("0.001"));
        assert_eq!(Kind::Number.render(&json!(10)).as_deref(), Some("10"));
        assert_eq!(Kind::Integer.render(&json!(64)).as_deref(), Some("64"));
        assert_eq!(
            Kind::Json.render(&json!({"a": [1]})).as_deref(),
            Some(r#"{"a":[1]}"#)
        );
        assert_eq!(Kind::Integer.render(&json!(1.5)), None);
        assert_eq!(Kind::String.render(&json!(3)), None);
        assert_eq!(Kind::Number.render(&Value::Null), None);
        let choice = Kind::Choice(vec![Choice::new(&json!("adam"), None).unwrap()]);
        assert_eq!(choice.render(&json!("adam")).as_deref(), Some("adam"));
        assert_eq!(choice.render(&json!("sgd")), None);
    }
}
