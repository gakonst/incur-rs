use serde_json::{Map, Value};

/// Filters a JSON value to comma-separated key paths.
pub(crate) fn apply(value: &Value, expression: &str) -> Value {
    let paths = split_paths(expression);
    if paths.is_empty() {
        return value.clone();
    }

    let paths = paths.iter().map(|path| parse_path(path)).collect::<Vec<_>>();
    if let [path] = paths.as_slice()
        && let [Segment::Key(key)] = path.as_slice()
    {
        if let Value::Array(values) = value {
            return Value::Array(values.iter().map(|value| apply(value, expression)).collect());
        }
        if let Some(selected) = value.as_object().and_then(|map| map.get(key)) {
            if selected.is_object() || selected.is_array() {
                return Value::Object(Map::from_iter([(key.clone(), selected.clone())]));
            }
            return selected.clone();
        }
        return Value::Null;
    }
    if let Value::Array(values) = value {
        return Value::Array(values.iter().map(|value| apply(value, expression)).collect());
    }

    let mut output = Map::new();
    for path in &paths {
        merge(&mut output, value, path, 0);
    }
    Value::Object(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Segment {
    Key(String),
    Slice(usize, Option<usize>),
}

fn split_paths(expression: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, character) in expression.char_indices() {
        match character {
            '[' => depth += 1,
            ']' => depth = usize::saturating_sub(depth, 1),
            ',' if depth == 0 => {
                let path = expression[start..index].trim();
                if !path.is_empty() {
                    paths.push(path.to_owned());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let path = expression[start..].trim();
    if !path.is_empty() {
        paths.push(path.to_owned());
    }
    paths
}

fn parse_path(path: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        let mut rest = part;
        if let Some(index) = rest.find('[') {
            if index > 0 {
                segments.push(Segment::Key(rest[..index].to_owned()));
            }
            while let Some(open) = rest.find('[') {
                let Some(close) = rest[open + 1..].find(']').map(|index| open + 1 + index) else {
                    break;
                };
                let range = &rest[open + 1..close];
                let mut bounds = range.splitn(2, ',');
                let start = bounds.next().and_then(|value| value.parse().ok()).unwrap_or(0);
                let end = bounds.next().and_then(|value| value.parse().ok());
                segments.push(Segment::Slice(start, end));
                rest = &rest[close + 1..];
            }
        } else if !rest.is_empty() {
            segments.push(Segment::Key(rest.to_owned()));
        }
    }
    segments
}

fn merge(target: &mut Map<String, Value>, data: &Value, segments: &[Segment], index: usize) {
    if index >= segments.len() {
        return;
    }
    let Segment::Key(key) = &segments[index] else {
        return;
    };
    let Some(value) = data.as_object().and_then(|map| map.get(key)) else {
        return;
    };
    if index + 1 >= segments.len() {
        target.insert(key.clone(), value.clone());
        return;
    }

    if let Segment::Slice(start, end) = segments[index + 1] {
        let Some(values) = value.as_array() else {
            return;
        };
        let end = end.unwrap_or(values.len()).min(values.len());
        let start = start.min(end);
        if index + 2 >= segments.len() {
            target.insert(key.clone(), Value::Array(values[start..end].to_vec()));
            return;
        }
        let selected = values[start..end]
            .iter()
            .map(|value| {
                let mut nested = Map::new();
                merge(&mut nested, value, segments, index + 2);
                Value::Object(nested)
            })
            .collect();
        target.insert(key.clone(), Value::Array(selected));
        return;
    }

    let nested = target.entry(key.clone()).or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(nested) = nested {
        merge(nested, value, segments, index + 1);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::apply;

    #[test]
    fn filters_nested_array_fields() {
        let value = json!({"users": [{"name": "A", "email": "a"}, {"name": "B", "email": "b"}]});
        assert_eq!(
            apply(&value, "users[0,2].name"),
            json!({"users": [{"name": "A"}, {"name": "B"}]})
        );
    }

    #[test]
    fn merges_multiple_paths() {
        let value = json!({"user": {"name": "A", "email": "a"}, "ignored": true});
        assert_eq!(
            apply(&value, "user.name,user.email"),
            json!({"user": {"name": "A", "email": "a"}})
        );
    }
}
