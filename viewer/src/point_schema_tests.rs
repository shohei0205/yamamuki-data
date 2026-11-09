use super::{Point, Points};
use serde_json::{Value, from_str, json};

const V5_SCHEMA: &str = include_str!("../../points/schemas/data-v5.schema.json");
const V6_SCHEMA: &str = include_str!("../../points/schemas/data-v6.schema.json");
const POINT_COMMON: &str = include_str!("../../points/schemas/point-common-v1.schema.json");
const METADATA: &str = include_str!("../../schemas/metadata-v1.schema.json");

fn examples(schema: &Value) -> Vec<Value> {
    if let Some(value) = schema.get("const") {
        return vec![value.clone()];
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let (file, pointer) = reference.split_once('#').expect("スキーマの参照先");
        let source = match file {
            "point-common-v1.schema.json" => POINT_COMMON,
            "../../schemas/metadata-v1.schema.json" => METADATA,
            "" => POINT_COMMON,
            _ => panic!("ビューワが読む項目に新しい参照先が追加されました: {reference}"),
        };
        let doc: Value = from_str(source).unwrap();
        return examples(doc.pointer(pointer).expect("スキーマの参照先"));
    }
    if let Some(alternatives) = schema["anyOf"].as_array() {
        return alternatives.iter().flat_map(examples).collect();
    }
    if let Some(types) = schema["type"].as_array() {
        return types
            .iter()
            .flat_map(|kind| {
                let mut candidate = schema.clone();
                candidate["type"] = kind.clone();
                examples(&candidate)
            })
            .collect();
    }
    match schema["type"].as_str() {
        Some("integer") => vec![json!(1)],
        Some("object") => {
            let fields = schema["properties"]
                .as_object()
                .expect("地点項目のプロパティ");
            let required = schema["required"]
                .as_array()
                .expect("地点項目の必須プロパティ");
            let baseline: serde_json::Map<String, Value> = required
                .iter()
                .map(|name| {
                    let name = name.as_str().unwrap();
                    (name.to_owned(), examples(&fields[name]).remove(0))
                })
                .collect();
            let mut output = vec![Value::Object(baseline.clone())];
            for (name, shape) in fields {
                for value in examples(shape) {
                    let mut object = baseline.clone();
                    object.insert(name.clone(), value);
                    output.push(Value::Object(object));
                }
            }
            output
        }
        Some("string") => vec![json!("富士山")],
        Some("number") => vec![json!(0), json!(0.5)],
        Some("null") => vec![Value::Null],
        Some("array") => examples(&schema["items"])
            .into_iter()
            .map(|item| json!([item]))
            .collect(),
        _ => panic!("ビューワが読む項目に新しい型が追加されました: {schema}"),
    }
}

#[test]
fn reads_every_schema_shape_for_display_fields() {
    for (version, source) in [(5, V5_SCHEMA), (6, V6_SCHEMA)] {
        let schema: Value = from_str(source).unwrap();
        let point_schema = &schema["$defs"]["point"];
        let required = point_schema["required"].as_array().unwrap();
        let sample = json!({"id":"gsi:1","name":"富士山","latitude":35.36,"longitude":138.73});
        for field in [
            "id",
            "name",
            "latitude",
            "longitude",
            "elevationM",
            "nameReading",
            "aliases",
            "tags",
            "graphic",
            "osmId",
            "relatedIds",
        ] {
            let shape = &point_schema["properties"][field];
            if shape.is_null() || shape == &Value::Bool(false) {
                continue;
            }
            let mut candidates = examples(shape).into_iter().map(Some).collect::<Vec<_>>();
            if !required.contains(&json!(field)) {
                candidates.push(None);
            }
            for candidate in candidates {
                let mut point = sample.clone();
                let fields = point.as_object_mut().unwrap();
                match candidate {
                    Some(value) => {
                        fields.insert(field.into(), value);
                    }
                    None => {
                        fields.remove(field);
                    }
                }
                let input = if version == 6 {
                    json!({
                        "schemaFormat": schema["properties"]["schemaFormat"]["const"],
                        "schemaVersion": schema["properties"]["schemaVersion"]["const"],
                        "points": [point]
                    })
                } else {
                    json!([point])
                };
                let data = Points::read(input.to_string().as_bytes())
                    .unwrap_or_else(|e| panic!("版{version} {field} {input}: {e:#}"));
                let _: &Point = data.point(0);
            }
        }
    }
}

#[test]
fn reads_the_repository_gsi_dataset() {
    let input = include_bytes!("../../points/gsi-peaks/gsi-peaks-source.json");
    let data = Points::read(input.as_slice()).unwrap();
    assert!(data.len() > 1_000);
}
