use crate::{Error, Result};
use serde_json::Value;

pub fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        let n = content.bytes().filter(|b| *b == b'\n').count();
        if content.as_bytes().last() == Some(&b'\n') {
            n
        } else {
            n + 1
        }
    }
}
pub fn validate_side(side: &str, change_type: Option<&Value>) -> Result<()> {
    if side != "left" && side != "right" {
        return Err(Error("Comment side must be left or right.".into()));
    }
    let (addition, deletion, description) = flags(change_type)?;
    if side == "left" && addition {
        return Err(Error(format!(
            "The left side does not exist for an Azure DevOps '{description}' change."
        )));
    }
    if side == "right" && deletion {
        return Err(Error(format!(
            "The right side does not exist for an Azure DevOps '{description}' change."
        )));
    }
    Ok(())
}
pub fn validate(
    side: &str,
    start: i64,
    end: i64,
    content: &str,
    change_type: Option<&Value>,
) -> Result<()> {
    validate_side(side, change_type)?;
    let count = line_count(content);
    if start <= 0 || end < start || end as usize > count {
        let unit = if count == 1 { "line" } else { "lines" };
        Err(Error(format!(
            "Comment line {end} is outside the selected file version, which has {count} {unit}."
        )))
    } else {
        Ok(())
    }
}
fn flags(raw: Option<&Value>) -> Result<(bool, bool, String)> {
    match raw {
        Some(Value::Number(n)) => {
            let v = n.as_i64().ok_or_else(missing)?;
            let undelete = v & 32 != 0;
            Ok((
                v & 1 != 0 || undelete,
                v & 16 != 0 && !undelete,
                v.to_string(),
            ))
        }
        Some(Value::String(s)) if !s.trim().is_empty() => {
            let tokens: Vec<String> = s
                .to_ascii_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect();
            let undelete = tokens.iter().any(|x| x == "undelete");
            Ok((
                tokens.iter().any(|x| x == "add") || undelete,
                tokens.iter().any(|x| x == "delete") && !undelete,
                s.clone(),
            ))
        }
        _ => Err(missing()),
    }
}
fn missing() -> Error {
    Error(
        "Azure DevOps did not return the change type needed to validate this comment anchor."
            .into(),
    )
}
