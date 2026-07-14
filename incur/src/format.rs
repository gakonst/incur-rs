use serde_json::{Map, Value};

#[cfg(feature = "toon")]
use crate::Error;
use crate::{InternalResult as Result, OutputFormat};

pub(crate) fn render(value: &Value, format: OutputFormat) -> Result<String> {
    if value.is_null() {
        return Ok(String::new());
    }

    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
        #[cfg(feature = "yaml")]
        OutputFormat::Yaml => Ok(serde_yaml::to_string(value)?.trim_end().to_owned()),
        OutputFormat::Markdown => Ok(markdown(value)),
        OutputFormat::Jsonl => jsonl(value),
        OutputFormat::Toon => toon(value),
    }
}

fn jsonl(value: &Value) -> Result<String> {
    if let Value::Array(values) = value {
        values
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|lines| lines.join("\n"))
            .map_err(Into::into)
    } else {
        serde_json::to_string(value).map_err(Into::into)
    }
}

#[cfg(feature = "toon")]
fn toon(value: &Value) -> Result<String> {
    if let Some(value) = scalar(value) {
        return Ok(value);
    }
    toon_format::encode_default(value).map_err(|error| Error::new("TOON_ERROR", error.to_string()))
}

#[cfg(not(feature = "toon"))]
fn toon(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some(String::new()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn markdown(value: &Value) -> String {
    match value {
        Value::Array(values) if array_is_table(values) => markdown_table(values),
        Value::Array(values) => values
            .iter()
            .map(|value| format!("- {}", markdown_inline(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => markdown_object(map, 2),
        _ => markdown_inline(value),
    }
}

fn markdown_object(map: &Map<String, Value>, level: usize) -> String {
    if map.values().all(is_scalar) {
        let rows = map
            .iter()
            .map(|(key, value)| format!("| {} | {} |", escape_cell(key), markdown_inline(value)))
            .collect::<Vec<_>>()
            .join("\n");
        return format!("| Key | Value |\n| --- | --- |\n{rows}");
    }

    map.iter()
        .map(|(key, value)| {
            let heading = "#".repeat(level);
            let rendered = match value {
                Value::Array(values) if array_is_table(values) => markdown_table(values),
                Value::Object(map) => markdown_object(map, level + 1),
                _ => markdown(value),
            };
            format!("{heading} {key}\n\n{rendered}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn array_is_table(values: &[Value]) -> bool {
    !values.is_empty()
        && values.iter().all(Value::is_object)
        && values.first().and_then(Value::as_object).is_some_and(|first| {
            values.iter().all(|value| {
                value
                    .as_object()
                    .is_some_and(|map| map.keys().eq(first.keys()) && map.values().all(is_scalar))
            })
        })
}

fn markdown_table(values: &[Value]) -> String {
    let Some(first) = values.first().and_then(Value::as_object) else {
        return String::new();
    };
    let keys = first.keys().collect::<Vec<_>>();
    let header =
        format!("| {} |", keys.iter().map(|key| escape_cell(key)).collect::<Vec<_>>().join(" | "));
    let separator = format!("| {} |", keys.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    let rows = values.iter().filter_map(Value::as_object).map(|row| {
        format!(
            "| {} |",
            keys.iter()
                .map(|key| markdown_inline(row.get(*key).unwrap_or(&Value::Null)))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    });
    std::iter::once(header)
        .chain(std::iter::once(separator))
        .chain(rows)
        .collect::<Vec<_>>()
        .join("\n")
}

const fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

fn markdown_inline(value: &Value) -> String {
    scalar(value)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
        .replace('\n', "<br>")
        .pipe(|value| escape_cell(&value))
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::render;
    use crate::OutputFormat;

    #[test]
    fn renders_core_output_formats() {
        let value = json!([{"name": "Ada", "active": true}, {"name": "Lin", "active": false}]);

        let json = render(&value, OutputFormat::Json).unwrap();
        assert!(json.contains("\"name\": \"Ada\""));
        let jsonl = render(&value, OutputFormat::Jsonl).unwrap();
        assert_eq!(jsonl.lines().count(), 2);
        let markdown = render(&value, OutputFormat::Markdown).unwrap();
        assert!(markdown.contains("| name | active |"));
        assert!(markdown.contains("| Ada | true |"));
    }

    #[cfg(feature = "toon")]
    #[test]
    fn renders_toon() {
        let output = render(&json!({"name": "Ada", "count": 2}), OutputFormat::Toon).unwrap();
        assert!(output.contains("name"));
        assert!(output.contains("Ada"));
    }

    #[cfg(not(feature = "toon"))]
    #[test]
    fn falls_back_to_json_without_toon() {
        let output = render(&json!({"name": "Ada"}), OutputFormat::Toon).unwrap();
        assert_eq!(output, r#"{"name":"Ada"}"#);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn renders_yaml() {
        let output = render(&json!({"name": "Ada"}), OutputFormat::Yaml).unwrap();
        assert_eq!(output, "name: Ada");
    }
}
