//! Project naming rules and case-insensitive collision checks.

use crate::context::{ContextError, NamingSettings, safe_output};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn format_texture_name(
    settings: &NamingSettings,
    asset_id: &str,
    surface: &str,
    channel: &str,
    width: u32,
    height: u32,
) -> Result<String, ContextError> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(ContextError::Invalid(
            "texture name dimensions must be 1..=8192".into(),
        ));
    }
    let values = BTreeMap::from([
        ("asset_id", safe_atom(asset_id)?),
        ("surface", safe_atom(surface)?),
        ("channel", safe_atom(channel)?),
        ("width", width.to_string()),
        ("height", height.to_string()),
    ]);
    format_pattern(&settings.texture_pattern, &values)
}

pub fn format_material_name(
    settings: &NamingSettings,
    asset_id: &str,
    surface: &str,
) -> Result<String, ContextError> {
    let values = BTreeMap::from([
        ("asset_id", safe_atom(asset_id)?),
        ("surface", safe_atom(surface)?),
    ]);
    format_pattern(&settings.material_pattern, &values)
}

/// Resolve a project-relative output path and reject existing names even when
/// only their letter casing differs (Windows and common engine targets).
pub fn check_name_collision(
    project_root: &Path,
    output_relative: &Path,
) -> Result<PathBuf, ContextError> {
    let output = safe_output(project_root, output_relative)?;
    let parent = output
        .parent()
        .ok_or_else(|| ContextError::UnsafePath(output_relative.display().to_string()))?;
    let requested = output
        .file_name()
        .ok_or_else(|| ContextError::UnsafePath(output_relative.display().to_string()))?
        .to_string_lossy();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(&requested)
        {
            return Err(ContextError::NameCollision(requested.into_owned()));
        }
    }
    Ok(output)
}

pub(crate) fn validate_pattern(
    pattern: &str,
    allowed: &[&str],
    required: &[&str],
) -> Result<(), ContextError> {
    let used = pattern_tokens(pattern)?;
    if used.iter().any(|token| !allowed.contains(&token.as_str())) {
        return Err(ContextError::Invalid(format!(
            "unsupported naming placeholder in `{pattern}`"
        )));
    }
    for token in required {
        if !used.iter().any(|used| used == token) {
            return Err(ContextError::Invalid(format!(
                "naming pattern `{pattern}` must include {{{token}}}"
            )));
        }
    }
    validate_filename(&render(
        pattern,
        &used.iter().map(|s| (s.as_str(), "x".to_owned())).collect(),
    )?)?;
    Ok(())
}

fn format_pattern(pattern: &str, values: &BTreeMap<&str, String>) -> Result<String, ContextError> {
    let output = render(pattern, values)?;
    validate_filename(&output)?;
    Ok(output)
}

fn pattern_tokens(pattern: &str) -> Result<Vec<String>, ContextError> {
    let mut tokens = Vec::new();
    let bytes = pattern.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                let close = bytes[index + 1..]
                    .iter()
                    .position(|byte| *byte == b'}')
                    .map(|offset| index + 1 + offset)
                    .ok_or_else(|| ContextError::Invalid("unclosed naming placeholder".into()))?;
                let token = &pattern[index + 1..close];
                if token.is_empty() || token.contains('{') {
                    return Err(ContextError::Invalid("malformed naming placeholder".into()));
                }
                tokens.push(token.to_owned());
                index = close + 1;
            }
            b'}' => {
                return Err(ContextError::Invalid("malformed naming placeholder".into()));
            }
            _ => index += 1,
        }
    }
    Ok(tokens)
}

fn render(pattern: &str, values: &BTreeMap<&str, String>) -> Result<String, ContextError> {
    pattern_tokens(pattern)?;
    let mut output = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        let close = rest
            .find('}')
            .ok_or_else(|| ContextError::Invalid("unclosed naming placeholder".into()))?;
        let token = &rest[..close];
        let value = values.get(token).ok_or_else(|| {
            ContextError::Invalid(format!("unsupported naming placeholder `{{{token}}}`"))
        })?;
        output.push_str(value);
        rest = &rest[close + 1..];
    }
    if rest.contains('}') {
        return Err(ContextError::Invalid("malformed naming placeholder".into()));
    }
    output.push_str(rest);
    Ok(output)
}

fn safe_atom(value: &str) -> Result<String, ContextError> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ContextError::UnsafePath(value.to_owned()));
    }
    Ok(value.to_owned())
}

fn validate_filename(name: &str) -> Result<(), ContextError> {
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    let device = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 240
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
        || reserved.contains(&device.as_str())
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.as_bytes()[3].is_ascii_digit()
            && device.as_bytes()[3] != b'0')
    {
        return Err(ContextError::UnsafePath(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::NamingSettings;

    #[test]
    fn formats_texture_and_material_names() {
        let settings = NamingSettings {
            texture_pattern: "T_{asset_id}_{surface}_{channel}_{width}x{height}.png".into(),
            material_pattern: "M_{asset_id}_{surface}".into(),
        };
        assert_eq!(
            format_texture_name(&settings, "prop_01", "wood", "basecolor", 128, 64).unwrap(),
            "T_prop_01_wood_basecolor_128x64.png"
        );
        assert_eq!(
            format_material_name(&settings, "prop_01", "wood").unwrap(),
            "M_prop_01_wood"
        );
        assert!(format_texture_name(&settings, "../prop", "wood", "basecolor", 128, 64).is_err());
    }

    #[test]
    fn rejects_malformed_and_unsafe_patterns() {
        assert!(
            validate_pattern(
                "T_{asset_id}}_{surface}_{channel}_{width}.png",
                &["asset_id", "surface", "channel", "width"],
                &["asset_id", "surface", "channel", "width"],
            )
            .is_err()
        );
        assert!(
            validate_pattern(
                "../T_{asset_id}_{surface}_{channel}_{width}.png",
                &["asset_id", "surface", "channel", "width"],
                &["asset_id", "surface", "channel", "width"],
            )
            .is_err()
        );
    }

    #[test]
    fn checks_case_insensitive_name_collisions() {
        let root = std::env::temp_dir().join(format!(
            "dd3d-name-collision-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let staging = root.join(".assetlab/staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("T_PROP_WOOD.png"), b"kept").unwrap();
        assert!(matches!(
            check_name_collision(&root, Path::new(".assetlab/staging/t_prop_wood.png")),
            Err(ContextError::NameCollision(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
